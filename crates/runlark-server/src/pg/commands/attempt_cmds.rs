use crate::auth::AuthenticatedUser;
use crate::error::ServerError;
use crate::pg::codec::{row_to_attempt, row_to_run, run_status_to_str, workflow_stage_to_str};
use crate::pg::commands::{TeamCommandResult, WorkspaceEventRecord, emit_workspace_event};
use runlark_domain::{
    Attempt, RecordAttemptCommand, Timestamp, TransitionError, WorkspaceId, WorkspaceSyncEvent,
    record_attempt_and_budget,
};

pub(crate) async fn exec_record_attempt(
    tx: &tokio_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    command_id: &str,
    cmd: RecordAttemptCommand,
    now: Timestamp,
    now_i64: i64,
) -> Result<TeamCommandResult, ServerError> {
    let dup = tx
        .query_opt(
            "SELECT 1 FROM team_attempts WHERE attempt_id = $1",
            &[&cmd.attempt_id.as_ref()],
        )
        .await?;
    if dup.is_some() {
        return Err(ServerError::conflict(
            "attempt_already_exists",
            format!("attempt {} already exists", cmd.attempt_id),
            None,
        ));
    }

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
        return Err(TransitionError::RunAlreadyTerminated.into());
    }

    let (prior_attempts, attempt) =
        build_and_insert_attempt(tx, workspace_id, &run.id, cmd, now, now_i64).await?;
    let stage_outcome = record_attempt_and_budget(&run, &prior_attempts, &attempt, now);
    run = stage_outcome.new_run;
    let cost_json = serde_json::to_string(&run.cost_summary)?;
    let next_rev = run.revision;
    tx.execute(
        concat!(
            "UPDATE team_runs SET status = $3, cost_summary_json = $4, revision = $5, updated_at = $6 ",
            "WHERE run_id = $1 AND workspace_id = $2",
        ),
        &[
            &run.id.as_ref(),
            &workspace_id.as_ref(),
            &run_status_to_str(run.status),
            &cost_json,
            &ServerError::to_pg_i64(next_rev.0)?,
            &now_i64,
        ],
    )
    .await?;

    let cursor =
        emit_attempt_and_run_events(tx, workspace_id, actor, &run, &ownership, attempt, now_i64)
            .await?;

    Ok(TeamCommandResult {
        command_id: command_id.to_string(),
        idempotent_replay: false,
        cursor_seq: cursor,
        aggregate_type: "run".to_string(),
        aggregate_id: run.id.0,
        revision: next_rev.0,
    })
}

async fn emit_attempt_and_run_events(
    tx: &tokio_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    run: &runlark_domain::Run,
    ownership: &runlark_domain::RunOwnership,
    attempt: Attempt,
    now_i64: i64,
) -> Result<u64, ServerError> {
    let attempt_id_str = attempt.id.0.clone();
    let _ = emit_workspace_event(
        tx,
        WorkspaceEventRecord {
            workspace_id,
            project_id: run.project_id.as_ref().map(|p| p.0.as_str()),
            aggregate_type: "attempt",
            aggregate_id: &attempt_id_str,
            event_type: "attempt_recorded",
            event: &WorkspaceSyncEvent::AttemptRecorded { attempt },
            actor_user_id: actor.user_id.as_ref(),
            created_at: now_i64,
        },
    )
    .await?;

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
                ownership: ownership.clone(),
            },
            actor_user_id: actor.user_id.as_ref(),
            created_at: now_i64,
        },
    )
    .await
}

async fn build_and_insert_attempt(
    tx: &tokio_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    run_id: &runlark_domain::RunId,
    cmd: RecordAttemptCommand,
    now: Timestamp,
    now_i64: i64,
) -> Result<(Vec<Attempt>, Attempt), ServerError> {
    let prior_rows = tx
        .query(
            concat!(
                "SELECT attempt_id, run_id, attempt_no, workflow_stage, started_at, ended_at, ",
                "outcome_json, cost_micro_usd, unknown_cost_reason, repair_reason ",
                "FROM team_attempts WHERE run_id = $1 ORDER BY attempt_no ASC",
            ),
            &[&run_id.as_ref()],
        )
        .await?;
    let mut prior_attempts = Vec::with_capacity(prior_rows.len());
    for pr in &prior_rows {
        prior_attempts.push(row_to_attempt(pr)?);
    }

    let next_attempt_no = i32::try_from(prior_attempts.len() + 1)
        .map_err(|_| ServerError::BadRequest("attempt count overflow".to_string()))?;
    let attempt = Attempt {
        id: cmd.attempt_id,
        run_id: run_id.clone(),
        attempt_no: ServerError::from_pg_i32(next_attempt_no, "attempt_no")?,
        workflow_stage: cmd.workflow_stage,
        started_at: now,
        ended_at: Some(now),
        outcome: cmd.outcome,
        cost_micro_usd: cmd.cost_micro_usd,
        unknown_cost_reason: cmd.unknown_cost_reason,
        repair_reason: cmd.repair_reason,
    };

    let outcome_json = serde_json::to_string(&attempt.outcome)?;
    tx.execute(
        concat!(
            "INSERT INTO team_attempts ",
            "(attempt_id, workspace_id, run_id, attempt_no, workflow_stage, started_at, ended_at, ",
            "outcome_json, cost_micro_usd, unknown_cost_reason, repair_reason) ",
            "VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        ),
        &[
            &attempt.id.as_ref(),
            &workspace_id.as_ref(),
            &run_id.as_ref(),
            &next_attempt_no,
            &workflow_stage_to_str(attempt.workflow_stage),
            &now_i64,
            &now_i64,
            &outcome_json,
            &attempt
                .cost_micro_usd
                .map(ServerError::to_pg_i64)
                .transpose()?,
            &attempt.unknown_cost_reason,
            &attempt.repair_reason,
        ],
    )
    .await?;

    Ok((prior_attempts, attempt))
}
