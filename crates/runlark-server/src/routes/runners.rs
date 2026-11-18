use crate::auth::authenticate_headers;
use crate::config::ServerConfig;
use crate::error::ServerError;
use crate::pg::{runner_fencing, runners};
use crate::server::ServerState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use runlark_domain::{
    Revision, Run, RunId, RunOwnership, RunnerId, RunnerRegistration, RunnerRunReport, Timestamp,
    WorkspaceId,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct RegisterRunnerRequest {
    pub runner_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct SweepRunnersRequest {
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct AssignRunnerRequest {
    pub runner_id: String,
    pub expected_revision: u64,
}

#[derive(Debug, Serialize)]
pub struct RunOwnershipResponse {
    pub run: Run,
    pub ownership: RunOwnership,
}

/// Lists all registered runners in `wid`.
///
/// # Errors
/// Returns [`ServerError`] on auth/membership denial.
pub async fn list_runners(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(wid): Path<String>,
) -> Result<Json<Vec<RunnerRegistration>>, ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let list = runners::list_runners(&state.pool, &user, &WorkspaceId::from(wid)).await?;
    Ok(Json(list))
}

/// Registers or refreshes a runner owned by the authenticated user.
///
/// # Errors
/// Returns [`ServerError`] on permission denial or runner ownership mismatch.
pub async fn register_runner(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(wid): Path<String>,
    Json(req): Json<RegisterRunnerRequest>,
) -> Result<Json<RunnerRegistration>, ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let now = Timestamp(ServerConfig::current_timestamp_ms());
    let runner = runners::register_runner(
        &state.pool,
        &user,
        &WorkspaceId::from(wid),
        req.runner_id.map(RunnerId::from),
        &req.name,
        now,
    )
    .await?;
    Ok(Json(runner))
}

/// Updates heartbeat liveness for an owned runner and sweeps stale peer runners in the
/// workspace whose heartbeat exceeded `runner_heartbeat_timeout_ms`.
///
/// # Errors
/// Returns [`ServerError`] if runner is missing or owned by another user.
pub async fn heartbeat_runner(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path((wid, rid)): Path<(String, String)>,
) -> Result<Json<RunnerRegistration>, ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let now_ms = ServerConfig::current_timestamp_ms();
    let stale_before = Timestamp(now_ms.saturating_sub(state.config.runner_heartbeat_timeout_ms));
    let runner = runners::record_heartbeat(
        &state.pool,
        &user,
        &WorkspaceId::from(wid),
        &RunnerId::from(rid),
        stale_before,
        Timestamp(now_ms),
    )
    .await?;
    Ok(Json(runner))
}

/// Marks timed-out runners `Unreachable` and flags their active runs without transferring ownership.
///
/// # Errors
/// Returns [`ServerError`] on auth denial or database error.
pub async fn sweep_runners(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(wid): Path<String>,
    body: Option<Json<SweepRunnersRequest>>,
) -> Result<Json<Vec<RunnerRegistration>>, ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let now_ms = ServerConfig::current_timestamp_ms();
    let timeout = body
        .and_then(|b| b.0.timeout_ms)
        .unwrap_or(state.config.runner_heartbeat_timeout_ms);
    let stale_before = Timestamp(now_ms.saturating_sub(timeout));
    let affected = runners::sweep_stale_runners(
        &state.pool,
        &WorkspaceId::from(wid),
        &user,
        stale_before,
        Timestamp(now_ms),
    )
    .await?;
    Ok(Json(affected))
}

/// Assigns a single `Online` runner to a run, refusing takeover of runs owned by another runner.
///
/// # Errors
/// Returns [`ServerError`] on permission denial, revision conflict, or active takeover block.
pub async fn assign_runner(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path((wid, run_id)): Path<(String, String)>,
    Json(req): Json<AssignRunnerRequest>,
) -> Result<Json<RunOwnershipResponse>, ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let now = Timestamp(ServerConfig::current_timestamp_ms());
    let (run, ownership) = runner_fencing::assign_runner(
        &state.pool,
        &user,
        &WorkspaceId::from(wid),
        &RunId::from(run_id),
        &RunnerId::from(req.runner_id),
        Revision(req.expected_revision),
        now,
    )
    .await?;

    Ok(Json(RunOwnershipResponse { run, ownership }))
}

/// Accepts fenced runner state updates and ordered run events from the owning runner.
///
/// # Errors
/// Returns [`ServerError`] (`409`) if `(runner_id, lease_epoch)` does not match current ownership.
pub async fn report_runner_state(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path((wid, run_id)): Path<(String, String)>,
    Json(report): Json<RunnerRunReport>,
) -> Result<Json<RunOwnershipResponse>, ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let now = Timestamp(ServerConfig::current_timestamp_ms());
    let (run, ownership) = runner_fencing::report_runner_state(
        &state.pool,
        &user,
        &WorkspaceId::from(wid),
        &RunId::from(run_id),
        report,
        now,
    )
    .await?;
    Ok(Json(RunOwnershipResponse { run, ownership }))
}
