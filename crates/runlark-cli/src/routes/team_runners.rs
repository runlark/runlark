use crate::routes::error::{ApiError, api_error, storage_error_to_http, team_server_error_to_http};
use crate::routes::team_workspaces::resolve_auth;
use crate::runner_agent::{RunOwnershipResponse, RunnerAgentWorker};
use crate::state::AppState;
use crate::team_http::validate_team_path_segment;
use crate::team_sync::{TeamCommandSubmissionResult, TeamSyncWorker, parse_server_json_response};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use runlark_domain::{
    CommandId, ForceCancelUnreachableRunCommand, Revision, RunId, RunnerId, RunnerRegistration,
    TeamCommand, Timestamp, WorkspaceId,
};
use runlark_local::{RunnerBufferStore, SessionStore, SyncStore, TeamAuthStore};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

#[derive(Debug, Deserialize, Default)]
pub struct WorkspaceQuery {
    pub workspace_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRunnerRequest {
    pub workspace_id: Option<String>,
    pub runner_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct LocalRunnerStatusResponse {
    pub workspace_id: Option<String>,
    pub local_runner_id: Option<String>,
    pub local_runner_name: Option<String>,
    pub buffered_events_count: usize,
    pub workspace_runners: Vec<RunnerRegistration>,
}

#[derive(Debug, Deserialize)]
pub struct AssignRunnerRequest {
    pub workspace_id: Option<String>,
    pub runner_id: String,
    pub expected_revision: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct CancelUnreachableRequest {
    pub workspace_id: Option<String>,
    pub command_id: Option<String>,
    pub reason: String,
    pub expected_revision: Option<u64>,
}

fn resolve_workspace_id(state: &AppState, explicit: Option<&str>) -> Result<WorkspaceId, ApiError> {
    if let Some(wid) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(WorkspaceId::from(wid));
    }
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    for sess in TeamAuthStore::list_sessions(&conn).map_err(storage_error_to_http)? {
        if let Some(wid) = sess.default_workspace_id {
            return Ok(wid);
        }
    }
    SyncStore::list_workspaces(&conn)
        .map_err(storage_error_to_http)?
        .into_iter()
        .next()
        .map(|w| w.workspace.id)
        .ok_or_else(|| {
            api_error(
                StatusCode::BAD_REQUEST,
                "workspace_required",
                "workspace_id is required (no default team workspace configured)",
            )
        })
}

/// Lists registered runners for a workspace (refreshing from server when online).
///
/// # Errors
/// Returns `400` if workspace cannot be resolved, `503` if unauthenticated.
pub async fn list_runners(
    State(state): State<AppState>,
    Query(query): Query<WorkspaceQuery>,
) -> Result<Json<Vec<RunnerRegistration>>, ApiError> {
    let wid = resolve_workspace_id(&state, query.workspace_id.as_deref())?;
    let wid_seg = validate_team_path_segment(wid.as_ref(), "workspace_id")
        .map_err(team_server_error_to_http)?;
    let auth = resolve_auth(&state, None, Some(&wid))?;
    let url = format!("{}/api/v1/workspaces/{wid_seg}/runners", auth.server_url);
    let worker = TeamSyncWorker::new(state.clone());
    if let Ok(resp) = worker
        .http()
        .get(&url)
        .bearer_auth(&auth.bearer_token)
        .send()
        .await
    {
        let runners: Vec<RunnerRegistration> = parse_server_json_response(resp)
            .await
            .map_err(team_server_error_to_http)?;
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        let now = Timestamp(AppState::current_timestamp_ms());
        SyncStore::ensure_workspace_placeholder(&conn, &auth.server_url, &wid, now)
            .map_err(storage_error_to_http)?;
        for r in &runners {
            SyncStore::upsert_runner(&conn, r).map_err(storage_error_to_http)?;
        }
        Ok(Json(runners))
    } else {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        let runners =
            SyncStore::list_runners_for_workspace(&conn, &wid).map_err(storage_error_to_http)?;
        Ok(Json(runners))
    }
}

/// Registers this local daemon as an execution runner on the team collaboration server.
///
/// # Errors
/// Returns `400` if name blank, `503` if server unreachable or unauthenticated.
pub async fn register_runner(
    State(state): State<AppState>,
    Json(req): Json<RegisterRunnerRequest>,
) -> Result<Json<RunnerRegistration>, ApiError> {
    if req.name.trim().is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_runner",
            "runner name is required",
        ));
    }
    let wid = resolve_workspace_id(&state, req.workspace_id.as_deref())?;
    let auth = resolve_auth(&state, None, Some(&wid))?;
    let agent = RunnerAgentWorker::new(state.clone());
    let reg = agent
        .register_runner(
            &auth.server_url,
            &auth.bearer_token,
            &wid,
            req.runner_id
                .filter(|s| !s.trim().is_empty())
                .map(RunnerId::from),
            req.name.trim(),
        )
        .await
        .map_err(team_server_error_to_http)?;
    state.notify_runner();
    Ok(Json(reg))
}

/// Returns local runner identity, buffered event queue depth, and workspace runners.
///
/// # Errors
/// Returns `500` on `SQLite` failure.
pub async fn runner_status(
    State(state): State<AppState>,
    Query(query): Query<WorkspaceQuery>,
) -> Result<Json<LocalRunnerStatusResponse>, ApiError> {
    let wid = resolve_workspace_id(&state, query.workspace_id.as_deref()).ok();
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let auth =
        TeamAuthStore::resolve_for_workspace(&conn, wid.as_ref()).map_err(storage_error_to_http)?;
    let buffered_events_count = RunnerBufferStore::list_unflushed(&conn, wid.as_ref())
        .map_err(storage_error_to_http)?
        .len();
    let workspace_runners = match &wid {
        Some(w) => {
            SyncStore::list_runners_for_workspace(&conn, w).map_err(storage_error_to_http)?
        }
        None => Vec::new(),
    };
    Ok(Json(LocalRunnerStatusResponse {
        workspace_id: wid.map(|w| w.0),
        local_runner_id: auth
            .as_ref()
            .and_then(|s| s.runner_id.as_ref())
            .map(|r| r.0.clone()),
        local_runner_name: auth.and_then(|s| s.runner_name),
        buffered_events_count,
        workspace_runners,
    }))
}

/// Assigns one `Online` runner to `run_id` via `runlark-server` (never transferring an active run).
///
/// # Errors
/// Returns `409` if already owned by another runner or revision mismatch, `503` if offline.
pub async fn assign_runner(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    Json(req): Json<AssignRunnerRequest>,
) -> Result<Json<RunOwnershipResponse>, ApiError> {
    let rid_seg =
        validate_team_path_segment(&run_id, "run_id").map_err(team_server_error_to_http)?;
    let rid = RunId::from(rid_seg);
    let wid = resolve_workspace_id(&state, req.workspace_id.as_deref())?;
    let wid_seg = validate_team_path_segment(wid.as_ref(), "workspace_id")
        .map_err(team_server_error_to_http)?;
    let auth = resolve_auth(&state, None, Some(&wid))?;
    let expected_revision = if let Some(rev) = req.expected_revision {
        rev
    } else {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        SessionStore::get_run(&conn, &rid)
            .map_err(storage_error_to_http)?
            .map_or(1, |r| r.revision.0)
    };
    let worker = TeamSyncWorker::new(state.clone());
    let url = format!(
        "{}/api/v1/workspaces/{wid_seg}/runs/{rid_seg}/assign",
        auth.server_url
    );
    let resp = worker
        .http()
        .post(&url)
        .bearer_auth(&auth.bearer_token)
        .json(&json!({
            "runner_id": req.runner_id,
            "expected_revision": expected_revision,
        }))
        .send()
        .await
        .map_err(|e| {
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "team_server_offline",
                e.to_string(),
            )
        })?;
    let out: RunOwnershipResponse = parse_server_json_response(resp)
        .await
        .map_err(team_server_error_to_http)?;
    let _ = worker
        .sync_workspace(&auth.server_url, &auth.bearer_token, &wid)
        .await;
    state.notify_scheduler();
    state.notify_runner();
    Ok(Json(out))
}

/// Explicitly cancels an unreachable runner's active run without silent auto-takeover.
///
/// # Errors
/// Returns `409` if runner is still reachable or revision mismatched, `503` if offline.
pub async fn cancel_unreachable_run(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    Json(req): Json<CancelUnreachableRequest>,
) -> Result<Json<TeamCommandSubmissionResult>, ApiError> {
    if req.reason.trim().is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_reason",
            "cancellation reason is required",
        ));
    }
    let rid = RunId::from(run_id);
    let wid = resolve_workspace_id(&state, req.workspace_id.as_deref())?;
    let auth = resolve_auth(&state, None, Some(&wid))?;
    let expected_revision = if let Some(rev) = req.expected_revision {
        Revision(rev)
    } else {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        SessionStore::get_run(&conn, &rid)
            .map_err(storage_error_to_http)?
            .map_or(Revision(1), |r| r.revision)
    };
    let command_id = CommandId::from(
        req.command_id
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("cmd_force_cancel_{}", Uuid::new_v4().simple())),
    );
    let now = Timestamp(AppState::current_timestamp_ms());
    let cmd = TeamCommand::ForceCancelUnreachableRun(ForceCancelUnreachableRunCommand {
        command_id,
        run_id: rid,
        reason: req.reason,
        expected_revision,
        created_at: now,
    });
    let res = TeamSyncWorker::new(state)
        .submit_command_and_sync(&auth.server_url, &auth.bearer_token, &wid, &cmd)
        .await
        .map_err(team_server_error_to_http)?;
    Ok(Json(res))
}
