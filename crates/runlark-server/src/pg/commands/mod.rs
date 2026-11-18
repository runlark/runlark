pub mod attempt_cmds;
pub mod comment_cmds;
pub mod review_cmds;
pub mod run_cmds;
pub mod task_cmds;

use crate::auth::{AuthenticatedUser, authorize_member};
use crate::cas::{ServerCasStore, sha256_hex};
use crate::error::ServerError;
use crate::pg::codec::row_to_task;
use crate::pg::runner_fencing::assign_runner_in_tx;
use crate::pg::{get_client, lock_workspace_write};
use deadpool_postgres::Pool;
use runlark_domain::{
    Revision, RunId, TaskId, TeamCommand, Timestamp, WorkspaceAction, WorkspaceId,
    WorkspaceSyncEvent,
};
use serde::Serialize;

/// Outcome of executing an ordered [`TeamCommand`] against `PostgreSQL`.
#[derive(Debug, Clone, Serialize)]
pub struct TeamCommandResult {
    pub command_id: String,
    pub idempotent_replay: bool,
    pub cursor_seq: u64,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub revision: u64,
}

pub(crate) struct WorkspaceEventRecord<'a> {
    pub workspace_id: &'a WorkspaceId,
    pub project_id: Option<&'a str>,
    pub aggregate_type: &'a str,
    pub aggregate_id: &'a str,
    pub event_type: &'a str,
    pub event: &'a WorkspaceSyncEvent,
    pub actor_user_id: &'a str,
    pub created_at: i64,
}

struct DispatchContext<'a> {
    cas: &'a ServerCasStore,
    workspace_id: &'a WorkspaceId,
    actor: &'a AuthenticatedUser,
    command_id: &'a str,
    now: Timestamp,
    now_i64: i64,
}

fn required_action(cmd: &TeamCommand) -> WorkspaceAction {
    match cmd {
        TeamCommand::RecordDecision(_) => WorkspaceAction::RecordDecision,
        TeamCommand::CreateTeamRun(_)
        | TeamCommand::AssignRunner(_)
        | TeamCommand::CancelRun(_)
        | TeamCommand::ForceCancelUnreachableRun(_) => WorkspaceAction::ManageRunners,
        TeamCommand::CreateTask(_)
        | TeamCommand::MoveTaskStage(_)
        | TeamCommand::UpdateTaskPlan(_)
        | TeamCommand::AddComment(_)
        | TeamCommand::RecordAttempt(_)
        | TeamCommand::AttachArtifact(_) => WorkspaceAction::MutateTasks,
    }
}

fn command_label(cmd: &TeamCommand) -> &'static str {
    match cmd {
        TeamCommand::CreateTask(_) => "create_task",
        TeamCommand::MoveTaskStage(_) => "move_task_stage",
        TeamCommand::UpdateTaskPlan(_) => "update_task_plan",
        TeamCommand::CreateTeamRun(_) => "create_team_run",
        TeamCommand::AssignRunner(_) => "assign_runner",
        TeamCommand::CancelRun(_) => "cancel_run",
        TeamCommand::ForceCancelUnreachableRun(_) => "force_cancel_unreachable_run",
        TeamCommand::AddComment(_) => "add_comment",
        TeamCommand::RecordAttempt(_) => "record_attempt",
        TeamCommand::RecordDecision(_) => "record_decision",
        TeamCommand::AttachArtifact(_) => "attach_artifact",
    }
}

pub(crate) async fn emit_workspace_event(
    tx: &tokio_postgres::Transaction<'_>,
    record: WorkspaceEventRecord<'_>,
) -> Result<u64, ServerError> {
    let payload_json = serde_json::to_string(record.event)?;
    let cursor: i64 = tx
        .query_one(
            concat!(
                "INSERT INTO workspace_events ",
                "(workspace_id, project_id, aggregate_type, aggregate_id, event_type, payload_json, actor_user_id, created_at) ",
                "VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ",
                "RETURNING cursor_seq",
            ),
            &[
                &record.workspace_id.as_ref(),
                &record.project_id,
                &record.aggregate_type,
                &record.aggregate_id,
                &record.event_type,
                &payload_json,
                &record.actor_user_id,
                &record.created_at,
            ],
        )
        .await?
        .get("cursor_seq");
    ServerError::from_pg_i64(cursor, "cursor_seq")
}

/// Clears `active_run_id` on `task_id` when `run_id` terminates, bumps task revision, and emits
/// `WorkspaceSyncEvent::TaskUpserted` so synced clients never retain stale active-run pointers.
pub(crate) async fn clear_task_active_run_and_emit(
    tx: &tokio_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    task_id: &TaskId,
    run_id: &RunId,
    actor_user_id: &str,
    now: Timestamp,
) -> Result<(), ServerError> {
    let row = tx
        .query_opt(
            concat!(
                "SELECT task_id, project_id, title, description, kind, stage, workflow_stage, ",
                "plan_text, plan_version, base_ref, head_ref, pr_url, active_session_id, ",
                "active_run_id, latest_run_id, budget_override_json, revision, created_at, updated_at ",
                "FROM team_tasks WHERE task_id = $1 AND workspace_id = $2 AND active_run_id = $3 FOR UPDATE",
            ),
            &[&task_id.as_ref(), &workspace_id.as_ref(), &run_id.as_ref()],
        )
        .await?;
    let Some(row) = row else {
        return Ok(());
    };

    let mut task = row_to_task(&row)?;
    let next_rev = Revision(task.revision.0.saturating_add(1));
    let now_i64 = ServerError::to_pg_i64(now.0)?;
    tx.execute(
        concat!(
            "UPDATE team_tasks ",
            "SET active_run_id = NULL, revision = $3, updated_at = $4 ",
            "WHERE task_id = $1 AND workspace_id = $2",
        ),
        &[
            &task.id.as_ref(),
            &workspace_id.as_ref(),
            &ServerError::to_pg_i64(next_rev.0)?,
            &now_i64,
        ],
    )
    .await?;

    task.active_run_id = None;
    task.revision = next_rev;
    task.updated_at = now;
    let project_id = task.project_id.0.clone();
    let task_id_str = task.id.0.clone();
    let _ = emit_workspace_event(
        tx,
        WorkspaceEventRecord {
            workspace_id,
            project_id: Some(&project_id),
            aggregate_type: "task",
            aggregate_id: &task_id_str,
            event_type: "task_upserted",
            event: &WorkspaceSyncEvent::TaskUpserted { task },
            actor_user_id,
            created_at: now_i64,
        },
    )
    .await?;
    Ok(())
}

/// Applies an ordered [`TeamCommand`] transactionally with RBAC check, `command_id`
/// idempotency/conflict validation, `expected_revision` OCC, and `workspace_events` emit.
///
/// # Errors
/// Returns [`ServerError`] on RBAC denial, command payload conflict, revision conflict,
/// invalid transition, or `PostgreSQL` failure.
pub async fn apply_team_command(
    pool: &Pool,
    cas: &ServerCasStore,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
    command: TeamCommand,
    now: Timestamp,
) -> Result<TeamCommandResult, ServerError> {
    let mut client = get_client(pool).await?;
    let canonical = command.canonical_for_hash();
    let canonical_json = serde_json::to_string(&canonical)?;
    let payload_hash = sha256_hex(canonical_json.as_bytes());
    let command_id = command.command_id().0.clone();
    let action = required_action(&command);

    let tx = client.transaction().await?;
    lock_workspace_write(&tx, workspace_id).await?;
    authorize_member(&tx, workspace_id, &actor.user_id, action).await?;
    if let Some(replay) =
        check_existing_command(&tx, workspace_id, &command_id, &payload_hash).await?
    {
        return Ok(replay);
    }

    let now_i64 = ServerError::to_pg_i64(now.0)?;
    let label = command_label(&command);
    let ctx = DispatchContext {
        cas,
        workspace_id,
        actor,
        command_id: &command_id,
        now,
        now_i64,
    };
    let result = dispatch_command_in_tx(&tx, &ctx, command).await?;

    let response_json = serde_json::json!({
        "cursor_seq": result.cursor_seq,
        "aggregate_type": result.aggregate_type,
        "aggregate_id": result.aggregate_id,
        "revision": result.revision,
    })
    .to_string();

    tx.execute(
        concat!(
            "INSERT INTO team_commands (workspace_id, command_id, command_type, payload_hash, response_json, created_at) ",
            "VALUES ($1, $2, $3, $4, $5, $6)",
        ),
        &[
            &workspace_id.as_ref(),
            &command_id,
            &label,
            &payload_hash,
            &response_json,
            &now_i64,
        ],
    )
    .await?;

    tx.commit().await?;
    Ok(result)
}

async fn check_existing_command(
    tx: &deadpool_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    command_id: &str,
    payload_hash: &str,
) -> Result<Option<TeamCommandResult>, ServerError> {
    let Some(prev) = tx
        .query_opt(
            "SELECT payload_hash, response_json FROM team_commands WHERE workspace_id = $1 AND command_id = $2",
            &[&workspace_id.as_ref(), &command_id],
        )
        .await?
    else {
        return Ok(None);
    };

    let stored_hash: String = prev.get("payload_hash");
    if stored_hash != payload_hash {
        return Err(ServerError::conflict(
            "command_payload_conflict",
            format!("command_id {command_id} already exists with different payload"),
            None,
        ));
    }
    let resp_json: String = prev.get("response_json");
    let parsed: serde_json::Value = serde_json::from_str(&resp_json)?;
    Ok(Some(TeamCommandResult {
        command_id: command_id.to_string(),
        idempotent_replay: true,
        cursor_seq: parsed["cursor_seq"].as_u64().unwrap_or(0),
        aggregate_type: parsed["aggregate_type"].as_str().unwrap_or("").to_string(),
        aggregate_id: parsed["aggregate_id"].as_str().unwrap_or("").to_string(),
        revision: parsed["revision"].as_u64().unwrap_or(0),
    }))
}

async fn dispatch_command_in_tx(
    tx: &deadpool_postgres::Transaction<'_>,
    ctx: &DispatchContext<'_>,
    command: TeamCommand,
) -> Result<TeamCommandResult, ServerError> {
    match command {
        TeamCommand::CreateTask(cmd) => {
            task_cmds::exec_create_task(tx, ctx.workspace_id, ctx.actor, cmd, ctx.now, ctx.now_i64)
                .await
        }
        TeamCommand::MoveTaskStage(cmd) => {
            task_cmds::exec_move_task_stage(
                tx,
                ctx.workspace_id,
                ctx.actor,
                cmd,
                ctx.now,
                ctx.now_i64,
            )
            .await
        }
        TeamCommand::UpdateTaskPlan(cmd) => {
            task_cmds::exec_update_task_plan(
                tx,
                ctx.workspace_id,
                ctx.actor,
                cmd,
                ctx.now,
                ctx.now_i64,
            )
            .await
        }
        TeamCommand::CreateTeamRun(cmd) => {
            run_cmds::exec_create_team_run(
                tx,
                ctx.workspace_id,
                ctx.actor,
                cmd,
                ctx.now,
                ctx.now_i64,
            )
            .await
        }
        TeamCommand::AssignRunner(cmd) => dispatch_assign_runner(tx, ctx, cmd).await,
        TeamCommand::CancelRun(cmd) => {
            run_cmds::exec_cancel_run(tx, ctx.workspace_id, ctx.actor, cmd, ctx.now, ctx.now_i64)
                .await
        }
        TeamCommand::ForceCancelUnreachableRun(cmd) => {
            run_cmds::exec_force_cancel_unreachable(
                tx,
                ctx.workspace_id,
                ctx.actor,
                cmd,
                ctx.now,
                ctx.now_i64,
            )
            .await
        }
        TeamCommand::RecordAttempt(cmd) => {
            attempt_cmds::exec_record_attempt(
                tx,
                ctx.workspace_id,
                ctx.actor,
                ctx.command_id,
                cmd,
                ctx.now,
                ctx.now_i64,
            )
            .await
        }
        TeamCommand::AddComment(_)
        | TeamCommand::RecordDecision(_)
        | TeamCommand::AttachArtifact(_) => dispatch_review_or_comment(tx, ctx, command).await,
    }
}

async fn dispatch_review_or_comment(
    tx: &deadpool_postgres::Transaction<'_>,
    ctx: &DispatchContext<'_>,
    command: TeamCommand,
) -> Result<TeamCommandResult, ServerError> {
    match command {
        TeamCommand::AddComment(cmd) => {
            comment_cmds::exec_add_comment(
                tx,
                ctx.workspace_id,
                ctx.actor,
                ctx.command_id,
                cmd,
                ctx.now,
                ctx.now_i64,
            )
            .await
        }
        TeamCommand::RecordDecision(cmd) => {
            review_cmds::exec_record_decision(
                tx,
                ctx.workspace_id,
                ctx.actor,
                ctx.command_id,
                cmd,
                ctx.now,
                ctx.now_i64,
            )
            .await
        }
        TeamCommand::AttachArtifact(cmd) => {
            review_cmds::exec_attach_artifact(
                tx,
                ctx.cas,
                ctx.workspace_id,
                ctx.actor,
                ctx.command_id,
                cmd,
                ctx.now_i64,
            )
            .await
        }
        _ => {
            unreachable!("only comment/decision/artifact commands reach dispatch_review_or_comment")
        }
    }
}

async fn dispatch_assign_runner(
    tx: &deadpool_postgres::Transaction<'_>,
    ctx: &DispatchContext<'_>,
    cmd: runlark_domain::AssignRunnerCommand,
) -> Result<TeamCommandResult, ServerError> {
    let (run, _) = assign_runner_in_tx(
        tx,
        ctx.workspace_id,
        &cmd.run_id,
        &cmd.runner_id,
        cmd.expected_revision,
        &ctx.actor.user_id,
        ctx.now,
    )
    .await?;
    let max_c: i64 = tx
        .query_one(
            "SELECT COALESCE(MAX(cursor_seq), 0) FROM workspace_events WHERE workspace_id = $1",
            &[&ctx.workspace_id.as_ref()],
        )
        .await?
        .get(0);
    Ok(TeamCommandResult {
        command_id: ctx.command_id.to_string(),
        idempotent_replay: false,
        cursor_seq: ServerError::from_pg_i64(max_c, "cursor_seq")?,
        aggregate_type: "run".to_string(),
        aggregate_id: run.id.0,
        revision: run.revision.0,
    })
}
