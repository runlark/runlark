use crate::routes::error::{ApiError, api_error, storage_error_to_http, team_server_error_to_http};
use crate::routes::team_workspaces::resolve_auth;
use crate::state::AppState;
use crate::team_sync::{TeamSyncSummary, TeamSyncWorker, parse_server_json_response};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use reqwest::Client;
use runlark_domain::{
    ProjectId, ProjectOwnershipMode, ServerPromotionAck, Timestamp, UserId, WorkspaceId,
    sanitize_project_for_team_snapshot,
};
use runlark_local::{
    DraftStore, ProjectSyncBinding, SyncOutboxStore, SyncStore, TeamAuthSession, TeamAuthStore,
    TeamWorkspaceRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Deserialize)]
pub struct TeamLoginRequest {
    pub server_url: String,
    pub bearer_token: String,
    pub workspace_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TeamLoginResponse {
    pub user_id: String,
    pub display_name: String,
    pub server_url: String,
    pub workspace_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TeamStatusResponse {
    pub sessions: Vec<TeamSessionSummary>,
    pub workspaces: Vec<TeamWorkspaceRecord>,
    pub project_bindings: Vec<ProjectSyncBinding>,
    pub unresolved_drafts_count: usize,
    pub pending_outbox_count: usize,
    pub conflict_outbox_count: usize,
}

#[derive(Debug, Serialize)]
pub struct TeamSessionSummary {
    pub server_url: String,
    pub user_id: String,
    pub display_name: String,
    pub default_workspace_id: Option<String>,
    pub runner_id: Option<String>,
    pub runner_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PromoteTeamRequest {
    pub server_url: Option<String>,
    pub workspace_id: String,
    pub bearer_token: Option<String>,
    pub ownership_epoch: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct PromoteTeamResponse {
    pub ack: ServerPromotionAck,
    pub binding: ProjectSyncBinding,
}

#[derive(Debug, Deserialize, Default)]
pub struct TeamSyncRequest {
    pub workspace_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WhoamiResponse {
    pub user_id: String,
    pub display_name: String,
}

/// Verifies a bearer token against `server_url` and persists the session locally.
///
/// # Errors
/// Returns `400` on invalid input, `401` if rejected, `503` if server unreachable.
pub async fn team_login(
    State(state): State<AppState>,
    Json(req): Json<TeamLoginRequest>,
) -> Result<Json<TeamLoginResponse>, ApiError> {
    let clean_url = req.server_url.trim().trim_end_matches('/').to_string();
    if clean_url.is_empty() || req.bearer_token.trim().is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_login",
            "server_url and bearer_token are required",
        ));
    }
    let http = Client::new();
    let resp = http
        .get(format!("{clean_url}/api/v1/auth/whoami"))
        .bearer_auth(req.bearer_token.trim())
        .send()
        .await
        .map_err(|e| {
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "team_server_offline",
                e.to_string(),
            )
        })?;
    let whoami: WhoamiResponse = parse_server_json_response(resp)
        .await
        .map_err(team_server_error_to_http)?;

    let ws_id = req
        .workspace_id
        .filter(|s| !s.trim().is_empty())
        .map(WorkspaceId::from);
    let worker = TeamSyncWorker::new(state.clone());
    let bearer_trimmed = req.bearer_token.trim().to_string();
    if let Some(ref wid) = ws_id {
        let _ = worker
            .fetch_snapshot(&clean_url, &bearer_trimmed, wid)
            .await
            .map_err(team_server_error_to_http)?;
    }

    let now = Timestamp(AppState::current_timestamp_ms());
    let session = TeamAuthSession {
        server_url: clean_url.clone(),
        user_id: UserId::from(whoami.user_id.clone()),
        display_name: whoami.display_name.clone(),
        bearer_token: bearer_trimmed,
        default_workspace_id: ws_id.clone(),
        runner_id: None,
        runner_name: None,
        updated_at: now,
    };
    let prev_session = {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        let prev =
            TeamAuthStore::get_by_server(&conn, &clean_url).map_err(storage_error_to_http)?;
        TeamAuthStore::save_session(&conn, &session).map_err(storage_error_to_http)?;
        prev
    };

    if let Some(ref wid) = ws_id
        && let Err(err) = worker
            .sync_workspace(&clean_url, &session.bearer_token, wid)
            .await
    {
        if let Ok(conn) = state.get_connection() {
            match prev_session {
                Some(ref prev) => {
                    let _ = TeamAuthStore::save_session(&conn, prev);
                }
                None => {
                    let _ = TeamAuthStore::delete_by_server(&conn, &clean_url);
                }
            }
        }
        return Err(team_server_error_to_http(err));
    }

    Ok(Json(TeamLoginResponse {
        user_id: whoami.user_id,
        display_name: whoami.display_name,
        server_url: clean_url,
        workspace_id: ws_id.map(|w| w.0),
    }))
}

/// Returns local team auth sessions, cached workspaces, project bindings, and outbox/draft counts.
///
/// # Errors
/// Returns `500` on `SQLite` failure.
pub async fn team_status(
    State(state): State<AppState>,
) -> Result<Json<TeamStatusResponse>, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let sessions = TeamAuthStore::list_sessions(&conn)
        .map_err(storage_error_to_http)?
        .into_iter()
        .map(|s| TeamSessionSummary {
            server_url: s.server_url,
            user_id: s.user_id.0,
            display_name: s.display_name,
            default_workspace_id: s.default_workspace_id.map(|w| w.0),
            runner_id: s.runner_id.map(|r| r.0),
            runner_name: s.runner_name,
        })
        .collect();
    let workspaces = SyncStore::list_workspaces(&conn).map_err(storage_error_to_http)?;
    let project_bindings =
        SyncStore::list_project_bindings(&conn).map_err(storage_error_to_http)?;
    let unresolved_drafts_count = DraftStore::list_unresolved_drafts(&conn, None)
        .map_err(storage_error_to_http)?
        .len();
    let entries = SyncOutboxStore::list_entries(&conn, None).map_err(storage_error_to_http)?;
    let pending_outbox_count = entries
        .iter()
        .filter(|e| e.status == runlark_local::SyncOutboxStatus::Pending)
        .count();
    let conflict_outbox_count = entries
        .iter()
        .filter(|e| {
            matches!(
                e.status,
                runlark_local::SyncOutboxStatus::Conflict | runlark_local::SyncOutboxStatus::Failed
            )
        })
        .count();

    Ok(Json(TeamStatusResponse {
        sessions,
        workspaces,
        project_bindings,
        unresolved_drafts_count,
        pending_outbox_count,
        conflict_outbox_count,
    }))
}

/// Promotes a local `SoloLocal` project to `TeamManaged` after validating zero active runs,
/// uploading the sanitized snapshot to `runlark-server`, and committing the `TeamManaged` binding.
///
/// # Errors
/// Returns `409` if any run is non-terminal or project is already promoted, `503` if server offline.
pub async fn promote_project_to_team(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<PromoteTeamRequest>,
) -> Result<Json<PromoteTeamResponse>, ApiError> {
    let project_id = ProjectId::from(id);
    let workspace_id = WorkspaceId::from(req.workspace_id);
    let epoch = req.ownership_epoch.unwrap_or(1).max(1);

    let (project, tasks, mut runs, decisions, server_url, bearer_token) = {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        let bundle = SyncStore::validate_project_promotion_eligible(&conn, &project_id)
            .map_err(storage_error_to_http)?;
        let auth = resolve_promote_auth(
            &conn,
            &workspace_id,
            req.server_url.as_deref(),
            req.bearer_token.as_deref(),
        )?;
        (
            bundle.project,
            bundle.tasks,
            bundle.runs,
            bundle.decisions,
            auth.0,
            auth.1,
        )
    };

    for run in &mut runs {
        run.worktree_path.clear();
    }
    let spec = sanitize_project_for_team_snapshot(&project, workspace_id.clone(), epoch);
    let worker = TeamSyncWorker::new(state.clone());
    let url = format!(
        "{server_url}/api/v1/workspaces/{}/projects/promote",
        workspace_id.as_ref()
    );
    let resp = worker
        .http()
        .post(&url)
        .bearer_auth(&bearer_token)
        .json(&json!({
            "project": spec,
            "tasks": tasks,
            "runs": runs,
            "decisions": decisions,
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
    let ack: ServerPromotionAck = parse_server_json_response(resp)
        .await
        .map_err(team_server_error_to_http)?;

    let now = Timestamp(AppState::current_timestamp_ms());
    let needs_auth_cache = {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        TeamAuthStore::get_by_server(&conn, &server_url)
            .map_err(storage_error_to_http)?
            .is_none()
    };
    if needs_auth_cache {
        cache_whoami_session(
            &state,
            &worker,
            &server_url,
            &bearer_token,
            &workspace_id,
            now,
        )
        .await;
    }
    let binding = {
        let mut conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        SyncStore::promote_project_to_team_atomic(&mut conn, &project_id, &server_url, &ack, now)
            .map_err(storage_error_to_http)?
    };
    let _ = worker
        .sync_workspace(&server_url, &bearer_token, &workspace_id)
        .await;
    Ok(Json(PromoteTeamResponse { ack, binding }))
}

fn resolve_promote_auth(
    conn: &runlark_local::Connection,
    workspace_id: &WorkspaceId,
    server_url: Option<&str>,
    bearer_token: Option<&str>,
) -> Result<(String, String), ApiError> {
    match (
        server_url.map(str::trim).filter(|s| !s.is_empty()),
        bearer_token.map(str::trim).filter(|s| !s.is_empty()),
    ) {
        (Some(u), Some(t)) => Ok((u.trim_end_matches('/').to_string(), t.to_string())),
        (Some(u), None) => {
            let s = TeamAuthStore::get_by_server(conn, u)
                .map_err(storage_error_to_http)?
                .ok_or_else(|| {
                    api_error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "team_auth_required",
                        format!("No team auth session for {u}; supply bearer_token or login first"),
                    )
                })?;
            Ok((s.server_url, s.bearer_token))
        }
        (None, opt_tok) => {
            let s = TeamAuthStore::resolve_for_workspace(conn, Some(workspace_id))
                .map_err(storage_error_to_http)?
                .ok_or_else(|| {
                    api_error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "team_auth_required",
                        "server_url and bearer_token (or prior `runlark team login`) are required",
                    )
                })?;
            Ok((s.server_url, opt_tok.map_or(s.bearer_token, str::to_string)))
        }
    }
}

async fn cache_whoami_session(
    state: &AppState,
    worker: &TeamSyncWorker,
    server_url: &str,
    bearer_token: &str,
    workspace_id: &WorkspaceId,
    now: Timestamp,
) {
    if let Ok(r) = worker
        .http()
        .get(format!("{server_url}/api/v1/auth/whoami"))
        .bearer_auth(bearer_token)
        .send()
        .await
        && let Ok(who) = parse_server_json_response::<WhoamiResponse>(r).await
        && let Ok(conn) = state.get_connection()
    {
        let _ = TeamAuthStore::save_session(
            &conn,
            &TeamAuthSession {
                server_url: server_url.to_string(),
                user_id: UserId::from(who.user_id),
                display_name: who.display_name,
                bearer_token: bearer_token.to_string(),
                default_workspace_id: Some(workspace_id.clone()),
                runner_id: None,
                runner_name: None,
                updated_at: now,
            },
        );
    }
}

/// Triggers an immediate synchronization pass for a workspace or all configured team workspaces.
///
/// # Errors
/// Returns `503` if server offline, or `500` on storage failure.
pub async fn trigger_team_sync(
    State(state): State<AppState>,
    body: Option<Json<TeamSyncRequest>>,
) -> Result<Json<TeamSyncSummary>, ApiError> {
    let worker = TeamSyncWorker::new(state.clone());
    let req = body.map(|b| b.0).unwrap_or_default();
    let summary = if let Some(wid_raw) = req.workspace_id.filter(|s| !s.trim().is_empty()) {
        let wid = WorkspaceId::from(wid_raw);
        let auth = resolve_auth(&state, None, Some(&wid))?;
        worker
            .sync_workspace(&auth.server_url, &auth.bearer_token, &wid)
            .await
            .map_err(team_server_error_to_http)?
    } else {
        worker
            .sync_once()
            .await
            .map_err(team_server_error_to_http)?
    };
    Ok(Json(summary))
}

/// Helper used across team routes to check if a project is `TeamManaged`.
pub(crate) fn is_project_team_managed(
    state: &AppState,
    project_id: &ProjectId,
) -> Result<Option<(WorkspaceId, String)>, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    match SyncStore::get_ownership_mode(&conn, project_id).map_err(storage_error_to_http)? {
        ProjectOwnershipMode::SoloLocal => Ok(None),
        ProjectOwnershipMode::TeamManaged {
            workspace_id,
            server_url,
            ..
        } => Ok(Some((workspace_id, server_url))),
    }
}
