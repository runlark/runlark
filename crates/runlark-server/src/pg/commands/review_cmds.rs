use crate::auth::AuthenticatedUser;
use crate::cas::ServerCasStore;
use crate::error::ServerError;
use crate::pg::codec::{
    decision_kind_to_str, row_to_artifact_ref, row_to_project, row_to_run, row_to_task,
    run_status_to_str, task_stage_to_str, workflow_stage_to_str,
};
use crate::pg::commands::{TeamCommandResult, WorkspaceEventRecord, emit_workspace_event};
use runlark_domain::{
    AttachArtifactCommand, Decision, DecisionBinding, DecisionKind, RecordDecisionCommand, Run,
    RunOwnership, RunStatus, Task, Timestamp, TransitionError, WorkspaceId, WorkspaceSyncEvent,
    apply_decision,
};

struct DecisionTxContext<'a> {
    workspace_id: &'a WorkspaceId,
    actor: &'a AuthenticatedUser,
    now_i64: i64,
}

struct DecisionAggregateState {
    task: Task,
    run: Run,
    ownership: RunOwnership,
}

pub(crate) async fn exec_record_decision(
    tx: &tokio_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    command_id: &str,
    cmd: RecordDecisionCommand,
    now: Timestamp,
    now_i64: i64,
) -> Result<TeamCommandResult, ServerError> {
    let dup = tx
        .query_opt(
            "SELECT 1 FROM team_decisions WHERE decision_id = $1",
            &[&cmd.decision_id.as_ref()],
        )
        .await?;
    if dup.is_some() {
        return Err(ServerError::conflict(
            "decision_already_exists",
            format!("decision {} already exists", cmd.decision_id),
            None,
        ));
    }

    let (task, run, ownership, policy_digest) =
        load_decision_context(tx, workspace_id, &cmd).await?;
    if run.status.is_terminal() {
        return Err(TransitionError::RunAlreadyTerminated.into());
    }
    if run.status == RunStatus::CancelRequested {
        return Err(TransitionError::InvalidRunState(format!(
            "cannot record decision on run {} while cancellation is requested",
            run.id
        ))
        .into());
    }

    let decision = Decision {
        id: cmd.decision_id.clone(),
        task_id: task.id.clone(),
        run_id: run.id.clone(),
        kind: cmd.kind,
        actor: actor.display_name.clone(),
        plan_version: cmd.plan_version,
        head_sha: cmd.head_sha,
        base_sha: cmd.base_sha,
        policy_digest: cmd.policy_digest,
        evidence_digest: cmd.evidence_digest,
        comment: cmd.comment,
        created_at: now,
    };

    let expected_head = run.head_sha.as_deref().unwrap_or("");
    let expected_base = run.base_sha.as_deref().unwrap_or("");
    let binding = DecisionBinding {
        plan_version: task.plan_version,
        head_sha: expected_head,
        base_sha: expected_base,
        policy_digest: &policy_digest,
        evidence_digest: &decision.evidence_digest,
    };
    let outcome = apply_decision(&task, &run, &decision, &binding, now)?;

    let ctx = DecisionTxContext {
        workspace_id,
        actor,
        now_i64,
    };
    let agg = DecisionAggregateState {
        task: outcome.new_task,
        run: outcome.new_run,
        ownership,
    };
    persist_decision_and_state(tx, &ctx, &agg, decision).await?;

    Ok(TeamCommandResult {
        command_id: command_id.to_string(),
        idempotent_replay: false,
        cursor_seq: emit_decision_task_event(tx, &ctx, &agg.task).await?,
        aggregate_type: "task".to_string(),
        aggregate_id: agg.task.id.0,
        revision: agg.task.revision.0,
    })
}

async fn load_decision_context(
    tx: &tokio_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    cmd: &RecordDecisionCommand,
) -> Result<(Task, Run, RunOwnership, runlark_domain::PolicyDigest), ServerError> {
    let task_row = tx
        .query_opt(
            concat!(
                "SELECT task_id, project_id, title, description, kind, stage, workflow_stage, ",
                "plan_text, plan_version, base_ref, head_ref, pr_url, active_session_id, ",
                "active_run_id, latest_run_id, budget_override_json, revision, created_at, updated_at ",
                "FROM team_tasks WHERE task_id = $1 AND workspace_id = $2 FOR UPDATE",
            ),
            &[&cmd.task_id.as_ref(), &workspace_id.as_ref()],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound(format!("task not found: {}", cmd.task_id)))?;
    let task = row_to_task(&task_row)?;
    if task.revision != cmd.expected_revision {
        return Err(ServerError::conflict(
            "revision_mismatch",
            format!(
                "task {} expected revision {}, actual {}",
                task.id, cmd.expected_revision.0, task.revision.0
            ),
            Some(task.revision.0),
        ));
    }

    let run_row = tx
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
    let (run, ownership) = row_to_run(&run_row)?;
    if run.task_id.as_ref() != Some(&task.id) {
        return Err(ServerError::BadRequest(format!(
            "run {} does not belong to task {}",
            run.id, task.id
        )));
    }

    let proj_row = tx
        .query_opt(
            concat!(
                "SELECT project_id, workspace_id, name, default_branch, max_concurrent_runs, ",
                "budget_policy_json, policy_digest, ownership_epoch, revision, created_at, updated_at ",
                "FROM team_projects WHERE project_id = $1",
            ),
            &[&task.project_id.as_ref()],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound(format!("project not found: {}", task.project_id)))?;
    let project = row_to_project(&proj_row)?;

    let linked_evidence = tx
        .query_opt(
            "SELECT 1 FROM team_run_artifacts WHERE run_id = $1 AND digest_sha256 = $2 LIMIT 1",
            &[&run.id.as_ref(), &cmd.evidence_digest.as_str()],
        )
        .await?
        .is_some();
    if cmd.kind == DecisionKind::Accept && !linked_evidence {
        return Err(ServerError::conflict(
            "evidence_required",
            format!(
                "evidence_digest {} must be attached to run {} before Accept",
                cmd.evidence_digest, run.id
            ),
            Some(task.revision.0),
        ));
    }

    let pinned_policy = run.policy_digest.clone().unwrap_or(project.policy_digest);
    Ok((task, run, ownership, pinned_policy))
}

async fn persist_decision_and_state(
    tx: &tokio_postgres::Transaction<'_>,
    ctx: &DecisionTxContext<'_>,
    agg: &DecisionAggregateState,
    decision: Decision,
) -> Result<(), ServerError> {
    tx.execute(
        concat!(
            "INSERT INTO team_decisions ",
            "(decision_id, workspace_id, task_id, run_id, kind, actor, plan_version, ",
            "head_sha, base_sha, policy_digest, evidence_digest, comment, created_at) ",
            "VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        ),
        &[
            &decision.id.as_ref(),
            &ctx.workspace_id.as_ref(),
            &decision.task_id.as_ref(),
            &decision.run_id.as_ref(),
            &decision_kind_to_str(decision.kind),
            &decision.actor,
            &ServerError::to_pg_i32(decision.plan_version)?,
            &decision.head_sha,
            &decision.base_sha,
            &decision.policy_digest.as_ref(),
            &decision.evidence_digest.as_str(),
            &decision.comment,
            &ctx.now_i64,
        ],
    )
    .await?;

    tx.execute(
        concat!(
            "UPDATE team_tasks ",
            "SET stage = $2, workflow_stage = $3, active_run_id = $4, revision = $5, updated_at = $6 ",
            "WHERE task_id = $1",
        ),
        &[
            &agg.task.id.as_ref(),
            &task_stage_to_str(agg.task.stage),
            &workflow_stage_to_str(agg.task.workflow_stage),
            &agg.task.active_run_id.as_ref().map(|r| r.0.as_str()),
            &ServerError::to_pg_i64(agg.task.revision.0)?,
            &ctx.now_i64,
        ],
    )
    .await?;

    tx.execute(
        concat!(
            "UPDATE team_runs ",
            "SET status = $2, workflow_stage = $3, revision = $4, updated_at = $5 ",
            "WHERE run_id = $1",
        ),
        &[
            &agg.run.id.as_ref(),
            &run_status_to_str(agg.run.status),
            &workflow_stage_to_str(agg.run.workflow_stage),
            &ServerError::to_pg_i64(agg.run.revision.0)?,
            &ctx.now_i64,
        ],
    )
    .await?;

    let decision_id_str = decision.id.0.clone();
    let run_id_str = agg.run.id.0.clone();
    let _ = emit_workspace_event(
        tx,
        WorkspaceEventRecord {
            workspace_id: ctx.workspace_id,
            project_id: Some(agg.task.project_id.as_ref()),
            aggregate_type: "decision",
            aggregate_id: &decision_id_str,
            event_type: "decision_recorded",
            event: &WorkspaceSyncEvent::DecisionRecorded { decision },
            actor_user_id: ctx.actor.user_id.as_ref(),
            created_at: ctx.now_i64,
        },
    )
    .await?;
    let _ = emit_workspace_event(
        tx,
        WorkspaceEventRecord {
            workspace_id: ctx.workspace_id,
            project_id: Some(agg.task.project_id.as_ref()),
            aggregate_type: "run",
            aggregate_id: &run_id_str,
            event_type: "run_upserted",
            event: &WorkspaceSyncEvent::RunUpserted {
                run: agg.run.clone(),
                ownership: agg.ownership.clone(),
            },
            actor_user_id: ctx.actor.user_id.as_ref(),
            created_at: ctx.now_i64,
        },
    )
    .await?;
    Ok(())
}

async fn emit_decision_task_event(
    tx: &tokio_postgres::Transaction<'_>,
    ctx: &DecisionTxContext<'_>,
    task: &Task,
) -> Result<u64, ServerError> {
    emit_workspace_event(
        tx,
        WorkspaceEventRecord {
            workspace_id: ctx.workspace_id,
            project_id: Some(task.project_id.as_ref()),
            aggregate_type: "task",
            aggregate_id: task.id.as_ref(),
            event_type: "task_upserted",
            event: &WorkspaceSyncEvent::TaskUpserted { task: task.clone() },
            actor_user_id: ctx.actor.user_id.as_ref(),
            created_at: ctx.now_i64,
        },
    )
    .await
}

pub(crate) async fn exec_attach_artifact(
    tx: &tokio_postgres::Transaction<'_>,
    cas: &ServerCasStore,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    command_id: &str,
    cmd: AttachArtifactCommand,
    now_i64: i64,
) -> Result<TeamCommandResult, ServerError> {
    let run_row = tx
        .query_opt(
            "SELECT project_id, revision FROM team_runs WHERE run_id = $1 AND workspace_id = $2 FOR UPDATE",
            &[&cmd.run_id.as_ref(), &workspace_id.as_ref()],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound(format!("run not found: {}", cmd.run_id)))?;
    let project_id: String = run_row.get("project_id");
    let run_revision = ServerError::from_pg_i64(run_row.get("revision"), "revision")?;

    let art_row = tx
        .query_opt(
            concat!(
                "SELECT a.digest_sha256, wa.kind, wa.media_type, a.size_bytes, wa.created_at ",
                "FROM team_artifacts a ",
                "INNER JOIN team_workspace_artifacts wa ON wa.digest_sha256 = a.digest_sha256 ",
                "WHERE wa.workspace_id = $1 AND a.digest_sha256 = $2",
            ),
            &[&workspace_id.as_ref(), &cmd.digest_sha256.as_str()],
        )
        .await?
        .ok_or_else(|| {
            ServerError::conflict(
                "artifact_not_uploaded",
                format!(
                    "artifact {} must be uploaded to workspace CAS before attaching",
                    cmd.digest_sha256
                ),
                Some(run_revision),
            )
        })?;
    let artifact = row_to_artifact_ref(&art_row)?;
    if artifact.kind != cmd.kind || artifact.size_bytes != cmd.size_bytes {
        return Err(ServerError::BadRequest(format!(
            "artifact metadata mismatch for {}",
            cmd.digest_sha256
        )));
    }
    let _ = cas.get_verified_bytes(&cmd.digest_sha256)?;

    tx.execute(
        concat!(
            "INSERT INTO team_run_artifacts (run_id, digest_sha256, label, created_at) ",
            "VALUES ($1, $2, $3, $4) ",
            "ON CONFLICT (run_id, digest_sha256, label) DO NOTHING",
        ),
        &[
            &cmd.run_id.as_ref(),
            &cmd.digest_sha256.as_str(),
            &cmd.label,
            &now_i64,
        ],
    )
    .await?;

    let digest_str = artifact.digest_sha256.as_str().to_string();
    let cursor = emit_workspace_event(
        tx,
        WorkspaceEventRecord {
            workspace_id,
            project_id: Some(&project_id),
            aggregate_type: "artifact",
            aggregate_id: &digest_str,
            event_type: "artifact_linked",
            event: &WorkspaceSyncEvent::ArtifactLinked {
                run_id: cmd.run_id.clone(),
                artifact,
                label: cmd.label,
            },
            actor_user_id: actor.user_id.as_ref(),
            created_at: now_i64,
        },
    )
    .await?;

    Ok(TeamCommandResult {
        command_id: command_id.to_string(),
        idempotent_replay: false,
        cursor_seq: cursor,
        aggregate_type: "run".to_string(),
        aggregate_id: cmd.run_id.0,
        revision: run_revision,
    })
}
