use crate::auth::{AuthenticatedUser, authorize_member};
use crate::error::ServerError;
use crate::pg::codec::{row_to_run, row_to_runner};
use crate::pg::{get_client, lock_workspace_write};
use deadpool_postgres::Pool;
use runlark_domain::{
    RunnerId, RunnerRegistration, RunnerStatus, Timestamp, WorkspaceAction, WorkspaceId,
    WorkspaceSyncEvent,
};
use uuid::Uuid;

/// Registers or refreshes a runner daemon owned by `actor` inside `workspace_id`.
///
/// # Errors
/// Returns [`ServerError`] on RBAC denial or `PostgreSQL` error.
pub async fn register_runner(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
    runner_id: Option<RunnerId>,
    name: &str,
    now: Timestamp,
) -> Result<RunnerRegistration, ServerError> {
    if name.trim().is_empty() {
        return Err(ServerError::BadRequest(
            "runner name must not be empty".to_string(),
        ));
    }
    let mut client = get_client(pool).await?;
    let rid =
        runner_id.unwrap_or_else(|| RunnerId::from(format!("runr_{}", Uuid::new_v4().simple())));
    let now_i64 = ServerError::to_pg_i64(now.0)?;
    let tx = client.transaction().await?;
    lock_workspace_write(&tx, workspace_id).await?;
    authorize_member(
        &tx,
        workspace_id,
        &actor.user_id,
        WorkspaceAction::ManageRunners,
    )
    .await?;
    let runner = upsert_runner_in_tx(&tx, workspace_id, actor, &rid, name, now, now_i64).await?;
    emit_runner_reconnect_events(&tx, workspace_id, actor, &runner, now_i64).await?;
    tx.commit().await?;
    Ok(runner)
}

async fn upsert_runner_in_tx(
    tx: &deadpool_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    rid: &RunnerId,
    name: &str,
    now: Timestamp,
    now_i64: i64,
) -> Result<RunnerRegistration, ServerError> {
    let existing = tx
        .query_opt(
            concat!(
                "SELECT runner_id, workspace_id, owner_user_id, name, status, last_heartbeat_at, created_at ",
                "FROM runners WHERE runner_id = $1 FOR UPDATE",
            ),
            &[&rid.as_ref()],
        )
        .await?;

    if let Some(row) = existing {
        let prev = row_to_runner(&row)?;
        if &prev.workspace_id != workspace_id || prev.owner_user_id != actor.user_id {
            return Err(ServerError::Forbidden(
                "runner_id belongs to another workspace or user".to_string(),
            ));
        }
        tx.execute(
            "UPDATE runners SET name = $2, status = 'online', last_heartbeat_at = $3 WHERE runner_id = $1",
            &[&rid.as_ref(), &name, &now_i64],
        )
        .await?;
        return Ok(RunnerRegistration {
            name: name.to_string(),
            status: RunnerStatus::Online,
            last_heartbeat_at: now,
            ..prev
        });
    }

    tx.execute(
        concat!(
            "INSERT INTO runners (runner_id, workspace_id, owner_user_id, name, status, last_heartbeat_at, created_at) ",
            "VALUES ($1, $2, $3, $4, 'online', $5, $5)",
        ),
        &[
            &rid.as_ref(),
            &workspace_id.as_ref(),
            &actor.user_id.as_ref(),
            &name,
            &now_i64,
        ],
    )
    .await?;
    Ok(RunnerRegistration {
        id: rid.clone(),
        workspace_id: workspace_id.clone(),
        owner_user_id: actor.user_id.clone(),
        name: name.to_string(),
        status: RunnerStatus::Online,
        last_heartbeat_at: now,
        created_at: now,
    })
}

async fn emit_runner_updated_event(
    tx: &deadpool_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    runner: &RunnerRegistration,
    now_i64: i64,
) -> Result<(), ServerError> {
    let ev_json = serde_json::to_string(&WorkspaceSyncEvent::RunnerUpdated {
        runner: runner.clone(),
    })?;
    tx.execute(
        concat!(
            "INSERT INTO workspace_events ",
            "(workspace_id, project_id, aggregate_type, aggregate_id, event_type, payload_json, actor_user_id, created_at) ",
            "VALUES ($1, NULL, 'runner', $2, 'runner_updated', $3, $4, $5)",
        ),
        &[
            &workspace_id.as_ref(),
            &runner.id.as_ref(),
            &ev_json,
            &actor.user_id.as_ref(),
            &now_i64,
        ],
    )
    .await?;
    Ok(())
}

async fn clear_unreachable_runs_and_emit(
    tx: &deadpool_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    runner_id: &RunnerId,
    now_i64: i64,
) -> Result<(), ServerError> {
    // Reconnecting an unreachable runner clears its unreachable flag without transferring
    // ownership or incrementing lease_epoch.
    let updated_runs = tx
        .query(
            concat!(
                "UPDATE team_runs ",
                "SET runner_unreachable = FALSE, updated_at = $2 ",
                "WHERE workspace_id = $1 ",
                "AND assigned_runner_id = $3 ",
                "AND runner_unreachable = TRUE ",
                "AND status NOT IN ('completed', 'failed', 'stopped') ",
                "RETURNING run_id, project_id, task_id, session_id, status, workflow_stage, task_kind, ",
                "workflow_digest, base_sha, head_sha, plan_version, policy_digest, budget_json, ",
                "cost_summary_json, repair_attempts, pr_url, assigned_runner_id, lease_epoch, ",
                "runner_unreachable, revision, created_at, updated_at",
            ),
            &[&workspace_id.as_ref(), &now_i64, &runner_id.as_ref()],
        )
        .await?;

    for row in &updated_runs {
        let (run, ownership) = row_to_run(row)?;
        let rev_json = serde_json::to_string(&WorkspaceSyncEvent::RunUpserted {
            run: run.clone(),
            ownership,
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
                &rev_json,
                &actor.user_id.as_ref(),
                &now_i64,
            ],
        )
        .await?;
    }
    Ok(())
}

async fn emit_runner_reconnect_events(
    tx: &deadpool_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    runner: &RunnerRegistration,
    now_i64: i64,
) -> Result<(), ServerError> {
    emit_runner_updated_event(tx, workspace_id, actor, runner, now_i64).await?;
    clear_unreachable_runs_and_emit(tx, workspace_id, actor, &runner.id, now_i64).await
}

/// Records a runner heartbeat, restoring `Online` status if it had drifted to `Unreachable`,
/// and sweeps stale peer runners in the workspace whose heartbeat expired before `stale_before`.
///
/// # Errors
/// Returns [`ServerError::NotFound`] or [`ServerError::Forbidden`] if owned by another user.
pub async fn record_heartbeat(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
    runner_id: &RunnerId,
    stale_before: Timestamp,
    now: Timestamp,
) -> Result<RunnerRegistration, ServerError> {
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
    let row = tx
        .query_opt(
            concat!(
                "SELECT runner_id, workspace_id, owner_user_id, name, status, last_heartbeat_at, created_at ",
                "FROM runners WHERE runner_id = $1 AND workspace_id = $2 FOR UPDATE",
            ),
            &[&runner_id.as_ref(), &workspace_id.as_ref()],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound(format!("runner not found: {runner_id}")))?;

    let prev = row_to_runner(&row)?;
    if prev.owner_user_id != actor.user_id {
        return Err(ServerError::Forbidden(
            "only the runner's owner may send heartbeats".to_string(),
        ));
    }

    let now_i64 = ServerError::to_pg_i64(now.0)?;
    let next_status = if prev.status == RunnerStatus::Drained {
        RunnerStatus::Drained
    } else {
        RunnerStatus::Online
    };

    tx.execute(
        "UPDATE runners SET status = $2, last_heartbeat_at = $3 WHERE runner_id = $1",
        &[&runner_id.as_ref(), &next_status.as_str(), &now_i64],
    )
    .await?;

    let updated = RunnerRegistration {
        status: next_status,
        last_heartbeat_at: now,
        ..prev.clone()
    };

    if prev.status != updated.status {
        emit_runner_updated_event(&tx, workspace_id, actor, &updated, now_i64).await?;
    }

    if updated.status == RunnerStatus::Online {
        clear_unreachable_runs_and_emit(&tx, workspace_id, actor, runner_id, now_i64).await?;
    }

    let _ = sweep_stale_runners_in_tx(
        &tx,
        workspace_id,
        actor.user_id.as_ref(),
        stale_before,
        now,
        Some(runner_id),
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}

/// Marks runners whose heartbeat expired as `Unreachable` and flags their active runs with
/// `runner_unreachable = true`, strictly preserving `assigned_runner_id` and `lease_epoch`.
///
/// # Errors
/// Returns [`ServerError`] if `PostgreSQL` update fails.
pub async fn sweep_stale_runners(
    pool: &Pool,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    stale_before: Timestamp,
    now: Timestamp,
) -> Result<Vec<RunnerRegistration>, ServerError> {
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
    let affected = sweep_stale_runners_in_tx(
        &tx,
        workspace_id,
        actor.user_id.as_ref(),
        stale_before,
        now,
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(affected)
}

/// Sweeps stale `Online` runners across all workspaces with expired heartbeats.
///
/// Invoked periodically by the server runtime loop so disconnected runners transition to
/// `Unreachable` without requiring a manual sweep request.
///
/// # Errors
/// Returns [`ServerError`] if querying or updating `PostgreSQL` fails.
pub async fn sweep_all_stale_runners(
    pool: &Pool,
    stale_before: Timestamp,
    now: Timestamp,
) -> Result<Vec<RunnerRegistration>, ServerError> {
    let cutoff_i64 = ServerError::to_pg_i64(stale_before.0)?;
    let mut client = get_client(pool).await?;
    let ws_rows = client
        .query(
            "SELECT DISTINCT workspace_id FROM runners WHERE status = 'online' AND last_heartbeat_at <= $1",
            &[&cutoff_i64],
        )
        .await?;

    let mut all_affected = Vec::new();
    for row in &ws_rows {
        let wid = WorkspaceId::from(row.get::<_, String>("workspace_id"));
        let tx = client.transaction().await?;
        lock_workspace_write(&tx, &wid).await?;
        let mut affected = sweep_stale_runners_in_tx(
            &tx,
            &wid,
            "system:heartbeat_sweeper",
            stale_before,
            now,
            None,
        )
        .await?;
        tx.commit().await?;
        all_affected.append(&mut affected);
    }
    Ok(all_affected)
}

async fn sweep_stale_runners_in_tx(
    tx: &deadpool_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor_user_id: &str,
    stale_before: Timestamp,
    now: Timestamp,
    exclude_runner_id: Option<&RunnerId>,
) -> Result<Vec<RunnerRegistration>, ServerError> {
    let cutoff_i64 = ServerError::to_pg_i64(stale_before.0)?;
    let now_i64 = ServerError::to_pg_i64(now.0)?;
    let excluded = exclude_runner_id.map(|r| r.0.as_str());

    let stale_rows = tx
        .query(
            concat!(
                "UPDATE runners ",
                "SET status = 'unreachable' ",
                "WHERE workspace_id = $1 AND status = 'online' AND last_heartbeat_at <= $2 ",
                "AND ($3::TEXT IS NULL OR runner_id <> $3) ",
                "RETURNING runner_id, workspace_id, owner_user_id, name, status, last_heartbeat_at, created_at",
            ),
            &[&workspace_id.as_ref(), &cutoff_i64, &excluded],
        )
        .await?;

    let mut affected = Vec::with_capacity(stale_rows.len());
    for row in &stale_rows {
        let runner = row_to_runner(row)?;
        let ev_json = serde_json::to_string(&WorkspaceSyncEvent::RunnerUpdated {
            runner: runner.clone(),
        })?;
        tx.execute(
            concat!(
                "INSERT INTO workspace_events ",
                "(workspace_id, project_id, aggregate_type, aggregate_id, event_type, payload_json, actor_user_id, created_at) ",
                "VALUES ($1, NULL, 'runner', $2, 'runner_updated', $3, $4, $5)",
            ),
            &[
                &workspace_id.as_ref(),
                &runner.id.as_ref(),
                &ev_json,
                &actor_user_id,
                &now_i64,
            ],
        )
        .await?;

        let flagged_runs = tx
            .query(
                concat!(
                    "UPDATE team_runs ",
                    "SET runner_unreachable = TRUE, updated_at = $3 ",
                    "WHERE workspace_id = $1 ",
                    "AND assigned_runner_id = $2 ",
                    "AND status NOT IN ('completed', 'failed', 'stopped') ",
                    "AND runner_unreachable = FALSE ",
                    "RETURNING run_id, project_id, task_id, session_id, status, workflow_stage, task_kind, ",
                    "workflow_digest, base_sha, head_sha, plan_version, policy_digest, budget_json, ",
                    "cost_summary_json, repair_attempts, pr_url, assigned_runner_id, lease_epoch, ",
                    "runner_unreachable, revision, created_at, updated_at",
                ),
                &[&workspace_id.as_ref(), &runner.id.as_ref(), &now_i64],
            )
            .await?;

        for r in &flagged_runs {
            let (run, ownership) = row_to_run(r)?;
            let run_ev = serde_json::to_string(&WorkspaceSyncEvent::RunUpserted {
                run: run.clone(),
                ownership,
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
                    &run_ev,
                    &actor_user_id,
                    &now_i64,
                ],
            )
            .await?;
        }
        affected.push(runner);
    }

    Ok(affected)
}

/// Lists all registered runners in `workspace_id`.
///
/// # Errors
/// Returns [`ServerError`] on RBAC denial or query failure.
pub async fn list_runners(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
) -> Result<Vec<RunnerRegistration>, ServerError> {
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
                "SELECT runner_id, workspace_id, owner_user_id, name, status, last_heartbeat_at, created_at ",
                "FROM runners WHERE workspace_id = $1 ORDER BY created_at ASC, runner_id ASC",
            ),
            &[&workspace_id.as_ref()],
        )
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        out.push(row_to_runner(r)?);
    }
    Ok(out)
}
