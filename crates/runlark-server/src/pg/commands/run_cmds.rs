use crate::auth::AuthenticatedUser;
use crate::error::ServerError;
use crate::pg::codec::{
    row_to_project, row_to_run, row_to_task, run_status_to_str, task_kind_to_str,
    workflow_stage_to_str,
};
use crate::pg::commands::{
    TeamCommandResult, WorkspaceEventRecord, clear_task_active_run_and_emit, emit_workspace_event,
};
use crate::pg::runner_fencing::assign_runner_in_tx;
use runlark_domain::{
    CancelTeamRunCommand, CostSummary, CreateTeamRunCommand, ForceCancelUnreachableRunCommand,
    LeaseEpoch, Revision, Run, RunOwnership, RunStatus, SessionId, Task, TeamProjectSpec,
    Timestamp, WorkspaceId, WorkspaceSyncEvent, start_task_run,
};

pub(crate) async fn exec_create_team_run(
    tx: &tokio_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    cmd: CreateTeamRunCommand,
    now: Timestamp,
    now_i64: i64,
) -> Result<TeamCommandResult, ServerError> {
    let existing_run = tx
        .query_opt(
            "SELECT 1 FROM team_runs WHERE run_id = $1",
            &[&cmd.run_id.as_ref()],
        )
        .await?;
    if existing_run.is_some() {
        return Err(ServerError::conflict(
            "run_already_exists",
            format!("run {} already exists", cmd.run_id),
            None,
        ));
    }

    let project = load_team_project(tx, &cmd.project_id, workspace_id).await?;
    let task = load_task_for_update(tx, &cmd.task_id, workspace_id).await?;
    if task.project_id != cmd.project_id {
        return Err(ServerError::BadRequest(format!(
            "task {} belongs to project {}, not {}",
            task.id, task.project_id, cmd.project_id
        )));
    }
    let session_id = SessionId::from(format!("sess_{}", cmd.run_id.0));
    let outcome = start_task_run(
        &task,
        &session_id,
        &cmd.run_id,
        task.active_run_id.is_some(),
        cmd.expected_revision,
        now,
    )?;
    let updated_task = outcome.new_task;

    let mut run = insert_team_run_row(
        tx,
        workspace_id,
        &project,
        &updated_task,
        &cmd,
        now,
        now_i64,
    )
    .await?;
    let task_project_id = updated_task.project_id.0.clone();
    let task_id_str = updated_task.id.0.clone();
    let _ = emit_workspace_event(
        tx,
        WorkspaceEventRecord {
            workspace_id,
            project_id: Some(&task_project_id),
            aggregate_type: "task",
            aggregate_id: &task_id_str,
            event_type: "task_upserted",
            event: &WorkspaceSyncEvent::TaskUpserted { task: updated_task },
            actor_user_id: actor.user_id.as_ref(),
            created_at: now_i64,
        },
    )
    .await?;

    let cursor = finalize_created_run(
        tx,
        workspace_id,
        actor,
        &mut run,
        cmd.runner_id,
        now,
        now_i64,
    )
    .await?;

    Ok(TeamCommandResult {
        command_id: cmd.command_id.0,
        idempotent_replay: false,
        cursor_seq: cursor,
        aggregate_type: "run".to_string(),
        aggregate_id: run.id.0,
        revision: run.revision.0,
    })
}

async fn finalize_created_run(
    tx: &tokio_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    run: &mut Run,
    runner_id: Option<runlark_domain::RunnerId>,
    now: Timestamp,
    now_i64: i64,
) -> Result<u64, ServerError> {
    if let Some(runner_id) = runner_id {
        let (assigned_run, _) = assign_runner_in_tx(
            tx,
            workspace_id,
            &run.id,
            &runner_id,
            Revision(1),
            &actor.user_id,
            now,
        )
        .await?;
        *run = assigned_run;
        let max_c: i64 = tx
            .query_one(
                "SELECT COALESCE(MAX(cursor_seq), 0) FROM workspace_events WHERE workspace_id = $1",
                &[&workspace_id.as_ref()],
            )
            .await?
            .get(0);
        return ServerError::from_pg_i64(max_c, "cursor_seq");
    }

    emit_workspace_event(
        tx,
        WorkspaceEventRecord {
            workspace_id,
            project_id: run.project_id.as_ref().map(|p| p.0.as_str()),
            aggregate_type: "run",
            aggregate_id: run.id.as_ref(),
            event_type: "run_upserted",
            event: &WorkspaceSyncEvent::RunUpserted {
                run: run.clone(),
                ownership: RunOwnership::default(),
            },
            actor_user_id: actor.user_id.as_ref(),
            created_at: now_i64,
        },
    )
    .await
}

async fn load_team_project(
    tx: &tokio_postgres::Transaction<'_>,
    project_id: &runlark_domain::ProjectId,
    workspace_id: &WorkspaceId,
) -> Result<TeamProjectSpec, ServerError> {
    let proj_row = tx
        .query_opt(
            concat!(
                "SELECT project_id, workspace_id, name, default_branch, max_concurrent_runs, ",
                "budget_policy_json, policy_digest, ownership_epoch, revision, created_at, updated_at ",
                "FROM team_projects WHERE project_id = $1 AND workspace_id = $2",
            ),
            &[&project_id.as_ref(), &workspace_id.as_ref()],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound(format!("project not found: {project_id}")))?;
    row_to_project(&proj_row)
}

async fn load_task_for_update(
    tx: &tokio_postgres::Transaction<'_>,
    task_id: &runlark_domain::TaskId,
    workspace_id: &WorkspaceId,
) -> Result<Task, ServerError> {
    let task_row = tx
        .query_opt(
            concat!(
                "SELECT task_id, project_id, title, description, kind, stage, workflow_stage, ",
                "plan_text, plan_version, base_ref, head_ref, pr_url, active_session_id, ",
                "active_run_id, latest_run_id, budget_override_json, revision, created_at, updated_at ",
                "FROM team_tasks WHERE task_id = $1 AND workspace_id = $2 FOR UPDATE",
            ),
            &[&task_id.as_ref(), &workspace_id.as_ref()],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound(format!("task not found: {task_id}")))?;
    row_to_task(&task_row)
}

async fn insert_team_run_row(
    tx: &tokio_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    project: &TeamProjectSpec,
    task: &Task,
    cmd: &CreateTeamRunCommand,
    now: Timestamp,
    now_i64: i64,
) -> Result<Run, ServerError> {
    let session_id = task
        .active_session_id
        .clone()
        .unwrap_or_else(|| SessionId::from(format!("sess_{}", cmd.run_id.0)));
    let budget = task
        .budget_override
        .clone()
        .unwrap_or_else(|| project.budget_policy.clone());
    let cost = CostSummary::default();
    let budget_json = serde_json::to_string(&budget)?;
    let cost_json = serde_json::to_string(&cost)?;
    let wf_stage = task.workflow_stage;

    tx.execute(
        concat!(
            "INSERT INTO team_runs ",
            "(run_id, workspace_id, project_id, task_id, session_id, status, workflow_stage, ",
            "task_kind, workflow_digest, base_sha, head_sha, plan_version, policy_digest, ",
            "budget_json, cost_summary_json, repair_attempts, pr_url, assigned_runner_id, ",
            "lease_epoch, runner_unreachable, revision, created_at, updated_at) ",
            "VALUES ($1, $2, $3, $4, $5, 'queued', $6, $7, NULL, NULL, NULL, $8, $9, $10, $11, 0, NULL, NULL, 0, FALSE, 1, $12, $12)",
        ),
        &[
            &cmd.run_id.as_ref(),
            &workspace_id.as_ref(),
            &cmd.project_id.as_ref(),
            &cmd.task_id.as_ref(),
            &session_id.as_ref(),
            &workflow_stage_to_str(wf_stage),
            &task_kind_to_str(task.kind),
            &ServerError::to_pg_i32(task.plan_version)?,
            &project.policy_digest.as_ref(),
            &budget_json,
            &cost_json,
            &now_i64,
        ],
    )
    .await?;

    tx.execute(
        concat!(
            "UPDATE team_tasks ",
            "SET stage = 'in_progress', workflow_stage = $2, active_run_id = $3, latest_run_id = $3, ",
            "active_session_id = $4, revision = $5, updated_at = $6 ",
            "WHERE task_id = $1",
        ),
        &[
            &task.id.as_ref(),
            &workflow_stage_to_str(wf_stage),
            &cmd.run_id.as_ref(),
            &session_id.as_ref(),
            &ServerError::to_pg_i64(task.revision.0)?,
            &now_i64,
        ],
    )
    .await?;

    Ok(Run {
        id: cmd.run_id.clone(),
        session_id,
        status: RunStatus::Queued,
        worktree_path: String::new(),
        revision: Revision(1),
        created_at: now,
        updated_at: now,
        task_id: Some(task.id.clone()),
        project_id: Some(cmd.project_id.clone()),
        workflow_stage: wf_stage,
        task_kind: task.kind,
        workflow_digest: None,
        base_sha: None,
        head_sha: None,
        plan_version: task.plan_version,
        policy_digest: Some(project.policy_digest.clone()),
        budget_policy: budget,
        cost_summary: cost,
        repair_attempts: 0,
        pr_url: None,
    })
}

pub(crate) async fn exec_cancel_run(
    tx: &tokio_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    cmd: CancelTeamRunCommand,
    now: Timestamp,
    now_i64: i64,
) -> Result<TeamCommandResult, ServerError> {
    let row = tx
        .query_opt(
            concat!(
                "SELECT run_id, project_id, task_id, session_id, status, workflow_stage, task_kind, ",
                "workflow_digest, base_sha, head_sha, plan_version, policy_digest, budget_json, ",
                "cost_summary_json, repair_attempts, pr_url, assigned_runner_id, lease_epoch, ",
                "runner_unreachable, revision, created_at, updated_at ",
                "FROM team_runs WHERE run_id = $1 AND workspace_id = $2 FOR UPDATE",
            ),
            &[&cmd.run_id.as_ref(), &workspace_id.as_ref()],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound(format!("run not found: {}", cmd.run_id)))?;
    let (mut run, ownership) = row_to_run(&row)?;
    if run.revision != cmd.expected_revision {
        return Err(ServerError::conflict(
            "revision_mismatch",
            format!(
                "run {} expected revision {}, actual {}",
                run.id, cmd.expected_revision.0, run.revision.0
            ),
            Some(run.revision.0),
        ));
    }
    if run.status.is_terminal() {
        return Err(ServerError::conflict(
            "run_already_terminal",
            format!("run {} is already terminal", run.id),
            Some(run.revision.0),
        ));
    }
    if ownership.runner_unreachable {
        return Err(ServerError::conflict(
            "runner_unreachable",
            format!(
                "run {} owner is unreachable; use ForceCancelUnreachableRun",
                run.id
            ),
            Some(run.revision.0),
        ));
    }

    // Cancel requested is never immediately Stopped when a runner holds an active lease.
    let next_status = if ownership.assigned_runner_id.is_none() && run.status == RunStatus::Queued {
        RunStatus::Stopped
    } else {
        RunStatus::CancelRequested
    };
    let next_rev = Revision(run.revision.0.saturating_add(1));

    tx.execute(
        "UPDATE team_runs SET status = $3, revision = $4, updated_at = $5 WHERE run_id = $1 AND workspace_id = $2",
        &[
            &run.id.as_ref(),
            &workspace_id.as_ref(),
            &run_status_to_str(next_status),
            &ServerError::to_pg_i64(next_rev.0)?,
            &now_i64,
        ],
    )
    .await?;

    if next_status == RunStatus::Stopped
        && let Some(ref tid) = run.task_id
    {
        clear_task_active_run_and_emit(tx, workspace_id, tid, &run.id, actor.user_id.as_ref(), now)
            .await?;
    }

    run.status = next_status;
    run.revision = next_rev;
    run.updated_at = now;
    let cursor = emit_workspace_event(
        tx,
        WorkspaceEventRecord {
            workspace_id,
            project_id: run.project_id.as_ref().map(|p| p.0.as_str()),
            aggregate_type: "run",
            aggregate_id: run.id.as_ref(),
            event_type: "run_upserted",
            event: &WorkspaceSyncEvent::RunUpserted {
                run: run.clone(),
                ownership,
            },
            actor_user_id: actor.user_id.as_ref(),
            created_at: now_i64,
        },
    )
    .await?;

    Ok(TeamCommandResult {
        command_id: cmd.command_id.0,
        idempotent_replay: false,
        cursor_seq: cursor,
        aggregate_type: "run".to_string(),
        aggregate_id: run.id.0,
        revision: next_rev.0,
    })
}

pub(crate) async fn exec_force_cancel_unreachable(
    tx: &tokio_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    cmd: ForceCancelUnreachableRunCommand,
    now: Timestamp,
    now_i64: i64,
) -> Result<TeamCommandResult, ServerError> {
    let row = tx
        .query_opt(
            concat!(
                "SELECT run_id, project_id, task_id, session_id, status, workflow_stage, task_kind, ",
                "workflow_digest, base_sha, head_sha, plan_version, policy_digest, budget_json, ",
                "cost_summary_json, repair_attempts, pr_url, assigned_runner_id, lease_epoch, ",
                "runner_unreachable, revision, created_at, updated_at ",
                "FROM team_runs WHERE run_id = $1 AND workspace_id = $2 FOR UPDATE",
            ),
            &[&cmd.run_id.as_ref(), &workspace_id.as_ref()],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound(format!("run not found: {}", cmd.run_id)))?;
    let (mut run, mut ownership) = row_to_run(&row)?;
    if run.revision != cmd.expected_revision {
        return Err(ServerError::conflict(
            "revision_mismatch",
            format!(
                "run {} expected revision {}, actual {}",
                run.id, cmd.expected_revision.0, run.revision.0
            ),
            Some(run.revision.0),
        ));
    }
    if !ownership.runner_unreachable {
        return Err(ServerError::conflict(
            "runner_still_reachable",
            format!("run {} owner is not marked unreachable", run.id),
            Some(run.revision.0),
        ));
    }
    if run.status.is_terminal() {
        return Err(ServerError::conflict(
            "run_already_terminal",
            format!("run {} is already terminal", run.id),
            Some(run.revision.0),
        ));
    }

    let next_rev = Revision(run.revision.0.saturating_add(1));
    let next_epoch = ownership.lease_epoch.0.saturating_add(1);
    tx.execute(
        concat!(
            "UPDATE team_runs ",
            "SET status = 'stopped', lease_epoch = $3, revision = $4, updated_at = $5 ",
            "WHERE run_id = $1 AND workspace_id = $2",
        ),
        &[
            &run.id.as_ref(),
            &workspace_id.as_ref(),
            &ServerError::to_pg_i64(next_epoch)?,
            &ServerError::to_pg_i64(next_rev.0)?,
            &now_i64,
        ],
    )
    .await?;

    if let Some(ref tid) = run.task_id {
        clear_task_active_run_and_emit(tx, workspace_id, tid, &run.id, actor.user_id.as_ref(), now)
            .await?;
    }

    run.status = RunStatus::Stopped;
    run.revision = next_rev;
    run.updated_at = now;
    ownership.lease_epoch = LeaseEpoch(next_epoch);

    let cursor = emit_workspace_event(
        tx,
        WorkspaceEventRecord {
            workspace_id,
            project_id: run.project_id.as_ref().map(|p| p.0.as_str()),
            aggregate_type: "run",
            aggregate_id: run.id.as_ref(),
            event_type: "run_upserted",
            event: &WorkspaceSyncEvent::RunUpserted {
                run: run.clone(),
                ownership,
            },
            actor_user_id: actor.user_id.as_ref(),
            created_at: now_i64,
        },
    )
    .await?;

    Ok(TeamCommandResult {
        command_id: cmd.command_id.0,
        idempotent_replay: false,
        cursor_seq: cursor,
        aggregate_type: "run".to_string(),
        aggregate_id: run.id.0,
        revision: next_rev.0,
    })
}
