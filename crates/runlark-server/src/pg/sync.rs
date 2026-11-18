use crate::auth::{AuthenticatedUser, authorize_member};
use crate::error::ServerError;
use crate::pg::codec::{
    row_to_artifact_ref, row_to_attempt, row_to_comment, row_to_decision, row_to_member,
    row_to_project, row_to_run, row_to_runner, row_to_task,
};
use crate::pg::{begin_repeatable_read, get_client};
use deadpool_postgres::{Pool, Transaction};
use runlark_domain::{
    Attempt, CursorSeq, Decision, EventSeq, LeaseEpoch, RunId, RunnerId, RunnerRegistration, Task,
    TaskComment, TaskId, TeamProjectSpec, TeamRunArtifactEntry, TeamRunEntry, TeamRunEvent,
    Timestamp, UserId, Workspace, WorkspaceAction, WorkspaceEventEnvelope, WorkspaceId,
    WorkspaceMember, WorkspaceSnapshot, WorkspaceSyncEvent,
};

/// Reads a consistent point-in-time [`WorkspaceSnapshot`] inside a `REPEATABLE READ` `PostgreSQL`
/// transaction so `cursor_seq` and all entities strictly agree.
///
/// # Errors
/// Returns [`ServerError`] on authorization failure or `PostgreSQL` query error.
pub async fn get_workspace_snapshot(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
) -> Result<WorkspaceSnapshot, ServerError> {
    let mut client = get_client(pool).await?;
    authorize_member(
        &client,
        workspace_id,
        &actor.user_id,
        WorkspaceAction::ReadWorkspace,
    )
    .await?;

    let tx = begin_repeatable_read(&mut client).await?;
    let workspace = load_workspace(&tx, workspace_id).await?;
    let max_cursor: i64 = tx
        .query_one(
            "SELECT COALESCE(MAX(cursor_seq), 0) FROM workspace_events WHERE workspace_id = $1",
            &[&workspace_id.as_ref()],
        )
        .await?
        .get(0);
    let cursor_seq = CursorSeq(ServerError::from_pg_i64(max_cursor, "cursor_seq")?);
    let members = load_members(&tx, workspace_id).await?;
    let projects = load_projects(&tx, workspace_id).await?;
    let tasks = load_tasks(&tx, workspace_id).await?;
    let runs = load_runs(&tx, workspace_id).await?;
    let attempts = load_attempts(&tx, workspace_id).await?;
    let decisions = load_decisions(&tx, workspace_id).await?;
    let comments = load_comments(&tx, workspace_id).await?;
    let artifacts = load_artifacts(&tx, workspace_id).await?;
    let runners = load_runners(&tx, workspace_id).await?;
    tx.commit().await?;

    Ok(WorkspaceSnapshot {
        cursor_seq,
        workspace,
        members,
        projects,
        tasks,
        runs,
        attempts,
        decisions,
        comments,
        artifacts,
        runners,
    })
}

async fn load_workspace(
    tx: &Transaction<'_>,
    workspace_id: &WorkspaceId,
) -> Result<Workspace, ServerError> {
    let ws_row = tx
        .query_opt(
            concat!(
                "SELECT workspace_id, name, slug, created_by, revision, created_at, updated_at ",
                "FROM workspaces WHERE workspace_id = $1",
            ),
            &[&workspace_id.as_ref()],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound(format!("workspace not found: {workspace_id}")))?;
    Ok(Workspace {
        id: WorkspaceId::from(ws_row.get::<_, String>("workspace_id")),
        name: ws_row.get("name"),
        slug: ws_row.get("slug"),
        created_by: UserId::from(ws_row.get::<_, String>("created_by")),
        revision: runlark_domain::Revision(ServerError::from_pg_i64(
            ws_row.get("revision"),
            "revision",
        )?),
        created_at: Timestamp(ServerError::from_pg_i64(
            ws_row.get("created_at"),
            "created_at",
        )?),
        updated_at: Timestamp(ServerError::from_pg_i64(
            ws_row.get("updated_at"),
            "updated_at",
        )?),
    })
}

async fn load_members(
    tx: &Transaction<'_>,
    workspace_id: &WorkspaceId,
) -> Result<Vec<WorkspaceMember>, ServerError> {
    let rows = tx
        .query(
            concat!(
                "SELECT wm.workspace_id, wm.user_id, u.display_name, wm.role, wm.joined_at ",
                "FROM workspace_members wm ",
                "INNER JOIN users u ON u.user_id = wm.user_id ",
                "WHERE wm.workspace_id = $1 ",
                "ORDER BY wm.joined_at ASC, wm.user_id ASC",
            ),
            &[&workspace_id.as_ref()],
        )
        .await?;
    rows.iter().map(row_to_member).collect()
}

async fn load_projects(
    tx: &Transaction<'_>,
    workspace_id: &WorkspaceId,
) -> Result<Vec<TeamProjectSpec>, ServerError> {
    let rows = tx
        .query(
            concat!(
                "SELECT project_id, workspace_id, name, default_branch, max_concurrent_runs, ",
                "budget_policy_json, policy_digest, ownership_epoch, revision, created_at, updated_at ",
                "FROM team_projects WHERE workspace_id = $1 ORDER BY created_at ASC",
            ),
            &[&workspace_id.as_ref()],
        )
        .await?;
    rows.iter().map(row_to_project).collect()
}

async fn load_tasks(
    tx: &Transaction<'_>,
    workspace_id: &WorkspaceId,
) -> Result<Vec<Task>, ServerError> {
    let rows = tx
        .query(
            concat!(
                "SELECT task_id, project_id, title, description, kind, stage, workflow_stage, ",
                "plan_text, plan_version, base_ref, head_ref, pr_url, active_session_id, ",
                "active_run_id, latest_run_id, budget_override_json, revision, created_at, updated_at ",
                "FROM team_tasks WHERE workspace_id = $1 ORDER BY created_at ASC, task_id ASC",
            ),
            &[&workspace_id.as_ref()],
        )
        .await?;
    rows.iter().map(row_to_task).collect()
}

async fn load_runs(
    tx: &Transaction<'_>,
    workspace_id: &WorkspaceId,
) -> Result<Vec<TeamRunEntry>, ServerError> {
    let rows = tx
        .query(
            concat!(
                "SELECT run_id, project_id, task_id, session_id, status, workflow_stage, task_kind, ",
                "workflow_digest, base_sha, head_sha, plan_version, policy_digest, budget_json, ",
                "cost_summary_json, repair_attempts, pr_url, assigned_runner_id, lease_epoch, ",
                "runner_unreachable, revision, created_at, updated_at ",
                "FROM team_runs WHERE workspace_id = $1 ORDER BY created_at ASC, run_id ASC",
            ),
            &[&workspace_id.as_ref()],
        )
        .await?;
    rows.iter()
        .map(|r| {
            let (run, ownership) = row_to_run(r)?;
            Ok(TeamRunEntry { run, ownership })
        })
        .collect()
}

async fn load_attempts(
    tx: &Transaction<'_>,
    workspace_id: &WorkspaceId,
) -> Result<Vec<Attempt>, ServerError> {
    let rows = tx
        .query(
            concat!(
                "SELECT attempt_id, run_id, attempt_no, workflow_stage, started_at, ended_at, ",
                "outcome_json, cost_micro_usd, unknown_cost_reason, repair_reason ",
                "FROM team_attempts WHERE workspace_id = $1 ORDER BY started_at ASC, attempt_no ASC",
            ),
            &[&workspace_id.as_ref()],
        )
        .await?;
    rows.iter().map(row_to_attempt).collect()
}

async fn load_decisions(
    tx: &Transaction<'_>,
    workspace_id: &WorkspaceId,
) -> Result<Vec<Decision>, ServerError> {
    let rows = tx
        .query(
            concat!(
                "SELECT decision_id, task_id, run_id, kind, actor, plan_version, head_sha, ",
                "base_sha, policy_digest, evidence_digest, comment, created_at ",
                "FROM team_decisions WHERE workspace_id = $1 ORDER BY created_at ASC, decision_id ASC",
            ),
            &[&workspace_id.as_ref()],
        )
        .await?;
    rows.iter().map(row_to_decision).collect()
}

async fn load_comments(
    tx: &Transaction<'_>,
    workspace_id: &WorkspaceId,
) -> Result<Vec<TaskComment>, ServerError> {
    let rows = tx
        .query(
            concat!(
                "SELECT comment_id, task_id, run_id, author_user_id, author_display_name, body, created_at ",
                "FROM team_comments WHERE workspace_id = $1 ORDER BY created_at ASC, comment_id ASC",
            ),
            &[&workspace_id.as_ref()],
        )
        .await?;
    rows.iter().map(row_to_comment).collect()
}

async fn load_artifacts(
    tx: &Transaction<'_>,
    workspace_id: &WorkspaceId,
) -> Result<Vec<TeamRunArtifactEntry>, ServerError> {
    let rows = tx
        .query(
            concat!(
                "SELECT ra.run_id, ra.label, a.digest_sha256, wa.kind, wa.media_type, a.size_bytes, wa.created_at ",
                "FROM team_run_artifacts ra ",
                "INNER JOIN team_runs r ON r.run_id = ra.run_id ",
                "INNER JOIN team_artifacts a ON a.digest_sha256 = ra.digest_sha256 ",
                "INNER JOIN team_workspace_artifacts wa ",
                "ON wa.workspace_id = r.workspace_id AND wa.digest_sha256 = ra.digest_sha256 ",
                "WHERE r.workspace_id = $1 ",
                "ORDER BY ra.created_at ASC",
            ),
            &[&workspace_id.as_ref()],
        )
        .await?;
    rows.iter()
        .map(|r| {
            Ok(TeamRunArtifactEntry {
                run_id: RunId::from(r.get::<_, String>("run_id")),
                artifact: row_to_artifact_ref(r)?,
                label: r.get("label"),
            })
        })
        .collect()
}

async fn load_runners(
    tx: &Transaction<'_>,
    workspace_id: &WorkspaceId,
) -> Result<Vec<RunnerRegistration>, ServerError> {
    let rows = tx
        .query(
            concat!(
                "SELECT runner_id, workspace_id, owner_user_id, name, status, last_heartbeat_at, created_at ",
                "FROM runners WHERE workspace_id = $1 ORDER BY created_at ASC, runner_id ASC",
            ),
            &[&workspace_id.as_ref()],
        )
        .await?;
    rows.iter().map(row_to_runner).collect()
}

/// Lists incremental ordered [`WorkspaceEventEnvelope`] records strictly after `after_cursor`.
///
/// # Errors
/// Returns [`ServerError`] on RBAC denial or query failure.
pub async fn list_workspace_events(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
    after_cursor: CursorSeq,
    limit: u32,
) -> Result<Vec<WorkspaceEventEnvelope>, ServerError> {
    let client = get_client(pool).await?;
    authorize_member(
        &client,
        workspace_id,
        &actor.user_id,
        WorkspaceAction::ReadWorkspace,
    )
    .await?;

    let clamped = i64::from(limit.clamp(1, 1000));
    let rows = client
        .query(
            concat!(
                "SELECT cursor_seq, workspace_id, project_id, aggregate_type, aggregate_id, ",
                "payload_json, actor_user_id, created_at ",
                "FROM workspace_events ",
                "WHERE workspace_id = $1 AND cursor_seq > $2 ",
                "ORDER BY cursor_seq ASC ",
                "LIMIT $3",
            ),
            &[
                &workspace_id.as_ref(),
                &ServerError::to_pg_i64(after_cursor.0)?,
                &clamped,
            ],
        )
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let payload_json: String = row.get("payload_json");
        let event: WorkspaceSyncEvent = serde_json::from_str(&payload_json)?;
        out.push(WorkspaceEventEnvelope {
            cursor_seq: CursorSeq(ServerError::from_pg_i64(
                row.get("cursor_seq"),
                "cursor_seq",
            )?),
            workspace_id: WorkspaceId::from(row.get::<_, String>("workspace_id")),
            project_id: row
                .get::<_, Option<String>>("project_id")
                .map(runlark_domain::ProjectId::from),
            aggregate_type: row.get("aggregate_type"),
            aggregate_id: row.get("aggregate_id"),
            event,
            actor_user_id: UserId::from(row.get::<_, String>("actor_user_id")),
            created_at: Timestamp(ServerError::from_pg_i64(
                row.get("created_at"),
                "created_at",
            )?),
        });
    }
    Ok(out)
}

/// Lists monotonic per-run events strictly after `after_seq`.
///
/// # Errors
/// Returns [`ServerError`] on RBAC denial, missing run, or query failure.
pub async fn list_run_events(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
    run_id: &RunId,
    after_seq: Option<EventSeq>,
) -> Result<Vec<TeamRunEvent>, ServerError> {
    let client = get_client(pool).await?;
    authorize_member(
        &client,
        workspace_id,
        &actor.user_id,
        WorkspaceAction::ReadWorkspace,
    )
    .await?;

    let run_exists = client
        .query_opt(
            "SELECT 1 FROM team_runs WHERE run_id = $1 AND workspace_id = $2",
            &[&run_id.as_ref(), &workspace_id.as_ref()],
        )
        .await?;
    if run_exists.is_none() {
        return Err(ServerError::NotFound(format!("run not found: {run_id}")));
    }

    let min_seq = match after_seq {
        Some(seq) => ServerError::to_pg_i64(seq.0)?,
        None => -1,
    };

    let rows = client
        .query(
            concat!(
                "SELECT run_id, run_seq, runner_id, lease_epoch, event_type, payload_json, created_at ",
                "FROM team_run_events ",
                "WHERE run_id = $1 AND run_seq > $2 ",
                "ORDER BY run_seq ASC",
            ),
            &[&run_id.as_ref(), &min_seq],
        )
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(TeamRunEvent {
            run_id: RunId::from(row.get::<_, String>("run_id")),
            seq: EventSeq(ServerError::from_pg_i64(row.get("run_seq"), "run_seq")?),
            runner_id: RunnerId::from(row.get::<_, String>("runner_id")),
            lease_epoch: LeaseEpoch(ServerError::from_pg_i64(
                row.get("lease_epoch"),
                "lease_epoch",
            )?),
            event_type: row.get("event_type"),
            payload_json: row.get("payload_json"),
            created_at: Timestamp(ServerError::from_pg_i64(
                row.get("created_at"),
                "created_at",
            )?),
        });
    }
    Ok(out)
}

/// Lists immutable append-only comments on a task.
///
/// # Errors
/// Returns [`ServerError`] on RBAC denial or query failure.
pub async fn list_task_comments(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
    task_id: &TaskId,
) -> Result<Vec<TaskComment>, ServerError> {
    let client = get_client(pool).await?;
    authorize_member(
        &client,
        workspace_id,
        &actor.user_id,
        WorkspaceAction::ReadWorkspace,
    )
    .await?;

    let rows = client
        .query(
            concat!(
                "SELECT comment_id, task_id, run_id, author_user_id, author_display_name, body, created_at ",
                "FROM team_comments ",
                "WHERE workspace_id = $1 AND task_id = $2 ",
                "ORDER BY created_at ASC, comment_id ASC",
            ),
            &[&workspace_id.as_ref(), &task_id.as_ref()],
        )
        .await?;
    rows.iter().map(row_to_comment).collect()
}
