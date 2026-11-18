use crate::auth::{AuthenticatedUser, authorize_member};
use crate::error::ServerError;
use crate::pg::codec::{row_to_run, row_to_runner, run_status_to_str, workflow_stage_to_str};
use crate::pg::commands::clear_task_active_run_and_emit;
use crate::pg::{get_client, lock_workspace_write};
use deadpool_postgres::Pool;
use runlark_domain::{
    LeaseEpoch, Revision, Run, RunId, RunOwnership, RunStatus, RunnerId, RunnerRegistration,
    RunnerRunReport, RunnerStatus, Timestamp, TransitionError, WorkflowStage, WorkspaceAction,
    WorkspaceId, WorkspaceSyncEvent, advance_run_stage, validate_runner_assignment,
    validate_runner_fencing,
};

/// Acquires the workspace write lock, verifies current `ManageRunners` authorization, and
/// assigns **one** `Online` runner to `run_id`.
///
/// # Errors
/// Returns [`ServerError`] on RBAC denial, missing run/runner, revision conflict, or takeover
/// rejection.
pub async fn assign_runner(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
    run_id: &RunId,
    runner_id: &RunnerId,
    expected_revision: Revision,
    now: Timestamp,
) -> Result<(Run, RunOwnership), ServerError> {
    let mut client = get_client(pool).await?;
    let tx = client.transaction().await?;
    lock_workspace_write(&tx, workspace_id).await?;
    authorize_member(
        &tx,
        workspace_id,
        &actor.user_id,
        WorkspaceAction::ManageRunners,
    )
    .await?;
    let result = assign_runner_in_tx(
        &tx,
        workspace_id,
        run_id,
        runner_id,
        expected_revision,
        &actor.user_id,
        now,
    )
    .await?;
    tx.commit().await?;
    Ok(result)
}

/// Assigns **one** `Online` runner to `run_id` inside an existing transaction, rejecting
/// automatic takeover if another runner already owns the run.
///
/// # Errors
/// Returns [`ServerError`] on missing run/runner, revision conflict, or takeover rejection.
pub async fn assign_runner_in_tx(
    tx: &tokio_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    run_id: &RunId,
    runner_id: &RunnerId,
    expected_revision: Revision,
    actor_user_id: &runlark_domain::UserId,
    now: Timestamp,
) -> Result<(Run, RunOwnership), ServerError> {
    lock_workspace_write(tx, workspace_id).await?;
    let run_row = tx
        .query_opt(
            concat!(
                "SELECT run_id, project_id, task_id, session_id, status, workflow_stage, task_kind, ",
                "workflow_digest, base_sha, head_sha, plan_version, policy_digest, budget_json, ",
                "cost_summary_json, repair_attempts, pr_url, assigned_runner_id, lease_epoch, ",
                "runner_unreachable, revision, created_at, updated_at ",
                "FROM team_runs WHERE run_id = $1 AND workspace_id = $2 FOR UPDATE",
            ),
            &[&run_id.as_ref(), &workspace_id.as_ref()],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound(format!("run not found: {run_id}")))?;
    let (mut run, ownership) = row_to_run(&run_row)?;
    if run.revision != expected_revision {
        return Err(ServerError::conflict(
            "revision_mismatch",
            format!(
                "run {} expected revision {}, actual {}",
                run_id, expected_revision.0, run.revision.0
            ),
            Some(run.revision.0),
        ));
    }

    let runner_row = tx
        .query_opt(
            concat!(
                "SELECT runner_id, workspace_id, owner_user_id, name, status, last_heartbeat_at, created_at ",
                "FROM runners WHERE runner_id = $1 AND workspace_id = $2 FOR UPDATE",
            ),
            &[&runner_id.as_ref(), &workspace_id.as_ref()],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound(format!("runner not found: {runner_id}")))?;
    let target_runner = row_to_runner(&runner_row)?;

    let next_epoch = validate_runner_assignment(run_id, run.status, &ownership, &target_runner)?;
    let next_rev = Revision(run.revision.0.saturating_add(1));
    let next_status = if run.status == RunStatus::Queued {
        RunStatus::Running
    } else {
        run.status
    };

    let now_i64 = ServerError::to_pg_i64(now.0)?;
    tx.execute(
        concat!(
            "UPDATE team_runs ",
            "SET assigned_runner_id = $3, lease_epoch = $4, runner_unreachable = FALSE, ",
            "status = $5, revision = $6, updated_at = $7 ",
            "WHERE run_id = $1 AND workspace_id = $2",
        ),
        &[
            &run_id.as_ref(),
            &workspace_id.as_ref(),
            &runner_id.as_ref(),
            &ServerError::to_pg_i64(next_epoch.0)?,
            &run_status_to_str(next_status),
            &ServerError::to_pg_i64(next_rev.0)?,
            &now_i64,
        ],
    )
    .await?;

    run.status = next_status;
    run.revision = next_rev;
    run.updated_at = now;
    let updated_ownership = RunOwnership {
        assigned_runner_id: Some(runner_id.clone()),
        lease_epoch: next_epoch,
        runner_unreachable: false,
    };

    let ev_json = serde_json::to_string(&WorkspaceSyncEvent::RunUpserted {
        run: run.clone(),
        ownership: updated_ownership.clone(),
    })?;
    tx.execute(
        concat!(
            "INSERT INTO workspace_events ",
            "(workspace_id, project_id, aggregate_type, aggregate_id, event_type, payload_json, actor_user_id, created_at) ",
            "VALUES ($1, $2, 'run', $3, 'run_upserted', $4, $5, $6)",
        ),
        &[
            &workspace_id.as_ref(),
            &run.project_id.as_ref().map(|p| p.0.as_str()),
            &run.id.as_ref(),
            &ev_json,
            &actor_user_id.as_ref(),
            &now_i64,
        ],
    )
    .await?;

    Ok((run, updated_ownership))
}

/// Applies fenced runner state updates and ordered `run_seq` events from the owning runner.
///
/// Rejects stale `(runner_id, lease_epoch)` writers, unreachable runners (requiring a heartbeat
/// back to `Online` first), terminal-run mutations, and illegal status or workflow-stage
/// transitions with `409 Conflict`.
///
/// # Errors
/// Returns [`ServerError`] on RBAC denial, fencing mismatch, unreachable runner, invalid
/// transition, or DB failure.
pub async fn report_runner_state(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
    run_id: &RunId,
    report: RunnerRunReport,
    now: Timestamp,
) -> Result<(Run, RunOwnership), ServerError> {
    let mut client = get_client(pool).await?;
    let tx = client.transaction().await?;
    lock_workspace_write(&tx, workspace_id).await?;
    authorize_member(
        &tx,
        workspace_id,
        &actor.user_id,
        WorkspaceAction::ManageRunners,
    )
    .await?;
    let runner =
        load_and_verify_runner_in_tx(&tx, workspace_id, &report.runner_id, &actor.user_id).await?;

    let run_row = tx
        .query_opt(
            concat!(
                "SELECT run_id, project_id, task_id, session_id, status, workflow_stage, task_kind, ",
                "workflow_digest, base_sha, head_sha, plan_version, policy_digest, budget_json, ",
                "cost_summary_json, repair_attempts, pr_url, assigned_runner_id, lease_epoch, ",
                "runner_unreachable, revision, created_at, updated_at ",
                "FROM team_runs WHERE run_id = $1 AND workspace_id = $2 FOR UPDATE",
            ),
            &[&run_id.as_ref(), &workspace_id.as_ref()],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound(format!("run not found: {run_id}")))?;

    let (mut run, ownership) = row_to_run(&run_row)?;
    validate_runner_fencing(
        run_id,
        &ownership,
        &report.runner_id,
        LeaseEpoch(report.lease_epoch),
    )?;
    if run.status.is_terminal() {
        return Err(TransitionError::RunAlreadyTerminated.into());
    }
    if runner.status == RunnerStatus::Unreachable || ownership.runner_unreachable {
        return Err(TransitionError::RunnerNotOnline {
            runner_id: runner.id,
            status: runner.status.as_str().to_string(),
        }
        .into());
    }

    let patch = FencedRunPatch {
        status: report.status,
        workflow_stage: report.workflow_stage,
        head_sha: report.head_sha.clone(),
        base_sha: report.base_sha.clone(),
    };
    let (next_status, next_stage) = compute_validated_runner_transition(&run, &patch, now)?;
    append_fenced_run_events(&tx, run_id, &report).await?;
    let resolved_patch = ResolvedRunPatch {
        next_status,
        next_stage,
        head_sha: patch.head_sha,
        base_sha: patch.base_sha,
    };
    apply_fenced_run_update(
        &tx,
        workspace_id,
        actor,
        &mut run,
        &ownership,
        resolved_patch,
        now,
    )
    .await?;

    tx.commit().await?;
    Ok((run, ownership))
}

fn validate_runner_status_transition(
    current: RunStatus,
    next: RunStatus,
) -> Result<(), ServerError> {
    if current == next {
        return Ok(());
    }
    let allowed = match current {
        RunStatus::Queued => matches!(
            next,
            RunStatus::Running
                | RunStatus::WaitingForApproval
                | RunStatus::Interrupted
                | RunStatus::Completed
                | RunStatus::Failed
                | RunStatus::Stopped
        ),
        RunStatus::Running => matches!(
            next,
            RunStatus::WaitingForApproval
                | RunStatus::Interrupted
                | RunStatus::Completed
                | RunStatus::Failed
                | RunStatus::Stopped
        ),
        RunStatus::WaitingForApproval => matches!(
            next,
            RunStatus::Running
                | RunStatus::Interrupted
                | RunStatus::Completed
                | RunStatus::Failed
                | RunStatus::Stopped
        ),
        RunStatus::Interrupted => matches!(
            next,
            RunStatus::Running
                | RunStatus::WaitingForApproval
                | RunStatus::Completed
                | RunStatus::Failed
                | RunStatus::Stopped
        ),
        RunStatus::CancelRequested => matches!(
            next,
            RunStatus::Stopped | RunStatus::Failed | RunStatus::Completed
        ),
        RunStatus::Stopped | RunStatus::Completed | RunStatus::Failed => false,
    };
    if !allowed {
        return Err(TransitionError::InvalidRunState(format!(
            "cannot transition run status from {current:?} to {next:?}"
        ))
        .into());
    }
    Ok(())
}

fn compute_validated_runner_transition(
    run: &Run,
    patch: &FencedRunPatch,
    now: Timestamp,
) -> Result<(RunStatus, WorkflowStage), ServerError> {
    let mut staged_status = run.status;
    let mut next_stage = run.workflow_stage;
    let mut entered_human_acceptance = false;
    if let Some(target_stage) = patch.workflow_stage
        && target_stage != run.workflow_stage
    {
        let outcome = advance_run_stage(run, target_stage, now)?;
        staged_status = outcome.new_run.status;
        next_stage = outcome.new_run.workflow_stage;
        entered_human_acceptance = target_stage == WorkflowStage::HumanAcceptance;
    }

    if let Some(requested_status) = patch.status {
        if entered_human_acceptance && requested_status != RunStatus::WaitingForApproval {
            return Err(TransitionError::InvalidRunState(format!(
                "entering HumanAcceptance sets WaitingForApproval, cannot combine with {requested_status:?}"
            ))
            .into());
        }
        validate_runner_status_transition(staged_status, requested_status)?;
        if run.status == RunStatus::CancelRequested && !requested_status.is_terminal() {
            return Err(TransitionError::InvalidRunState(
                "CancelRequested run can only settle to terminal status".to_string(),
            )
            .into());
        }
        staged_status = requested_status;
    }
    Ok((staged_status, next_stage))
}

async fn load_and_verify_runner_in_tx(
    tx: &deadpool_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    runner_id: &RunnerId,
    actor_user_id: &runlark_domain::UserId,
) -> Result<RunnerRegistration, ServerError> {
    let runner_row = tx
        .query_opt(
            concat!(
                "SELECT runner_id, workspace_id, owner_user_id, name, status, last_heartbeat_at, created_at ",
                "FROM runners WHERE runner_id = $1 AND workspace_id = $2 FOR UPDATE",
            ),
            &[&runner_id.as_ref(), &workspace_id.as_ref()],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound(format!("runner not found: {runner_id}")))?;
    let runner = row_to_runner(&runner_row)?;
    if &runner.owner_user_id != actor_user_id {
        return Err(ServerError::Forbidden(
            "runner does not belong to authenticated caller".to_string(),
        ));
    }
    Ok(runner)
}

struct FencedRunPatch {
    status: Option<RunStatus>,
    workflow_stage: Option<WorkflowStage>,
    head_sha: Option<String>,
    base_sha: Option<String>,
}

struct ResolvedRunPatch {
    next_status: RunStatus,
    next_stage: WorkflowStage,
    head_sha: Option<String>,
    base_sha: Option<String>,
}

async fn apply_fenced_run_update(
    tx: &deadpool_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    run: &mut Run,
    ownership: &RunOwnership,
    patch: ResolvedRunPatch,
    now: Timestamp,
) -> Result<(), ServerError> {
    let now_i64 = ServerError::to_pg_i64(now.0)?;
    let next_status = patch.next_status;
    let next_stage = patch.next_stage;
    let next_head = patch.head_sha.or_else(|| run.head_sha.clone());
    let next_base = patch.base_sha.or_else(|| run.base_sha.clone());
    let next_rev = Revision(run.revision.0.saturating_add(1));

    tx.execute(
        concat!(
            "UPDATE team_runs ",
            "SET status = $3, workflow_stage = $4, head_sha = $5, base_sha = $6, ",
            "revision = $7, updated_at = $8 ",
            "WHERE run_id = $1 AND workspace_id = $2",
        ),
        &[
            &run.id.as_ref(),
            &workspace_id.as_ref(),
            &run_status_to_str(next_status),
            &workflow_stage_to_str(next_stage),
            &next_head,
            &next_base,
            &ServerError::to_pg_i64(next_rev.0)?,
            &now_i64,
        ],
    )
    .await?;

    if next_status.is_terminal()
        && let Some(task_id) = &run.task_id
    {
        clear_task_active_run_and_emit(
            tx,
            workspace_id,
            task_id,
            &run.id,
            actor.user_id.as_ref(),
            now,
        )
        .await?;
    }

    run.status = next_status;
    run.workflow_stage = next_stage;
    run.head_sha = next_head;
    run.base_sha = next_base;
    run.revision = next_rev;
    run.updated_at = now;

    let ev_json = serde_json::to_string(&WorkspaceSyncEvent::RunUpserted {
        run: run.clone(),
        ownership: ownership.clone(),
    })?;
    tx.execute(
        concat!(
            "INSERT INTO workspace_events ",
            "(workspace_id, project_id, aggregate_type, aggregate_id, event_type, payload_json, actor_user_id, created_at) ",
            "VALUES ($1, $2, 'run', $3, 'run_upserted', $4, $5, $6)",
        ),
        &[
            &workspace_id.as_ref(),
            &run.project_id.as_ref().map(|p| p.0.as_str()),
            &run.id.as_ref(),
            &ev_json,
            &actor.user_id.as_ref(),
            &now_i64,
        ],
    )
    .await?;
    Ok(())
}

async fn append_fenced_run_events(
    tx: &deadpool_postgres::Transaction<'_>,
    run_id: &RunId,
    report: &RunnerRunReport,
) -> Result<(), ServerError> {
    for ev in &report.events {
        tx.execute(
            concat!(
                "INSERT INTO team_run_events ",
                "(run_id, run_seq, runner_id, lease_epoch, event_type, payload_json, created_at) ",
                "VALUES ($1, $2, $3, $4, $5, $6, $7) ",
                "ON CONFLICT (run_id, run_seq) DO NOTHING",
            ),
            &[
                &run_id.as_ref(),
                &ServerError::to_pg_i64(ev.seq.0)?,
                &report.runner_id.as_ref(),
                &ServerError::to_pg_i64(report.lease_epoch)?,
                &ev.event_type,
                &ev.payload_json,
                &ServerError::to_pg_i64(ev.created_at.0)?,
            ],
        )
        .await?;
    }
    Ok(())
}
