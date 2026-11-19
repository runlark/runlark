use crate::routes::error::{ApiError, api_error, storage_error_to_http, team_server_error_to_http};
use crate::state::AppState;
use crate::team_http::validate_team_path_segment;
use crate::team_sync::{TeamSyncWorker, parse_server_json_response};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use runlark_domain::{
    Timestamp, Workspace, WorkspaceId, WorkspaceInvitation, WorkspaceMember, WorkspaceRole,
};
use runlark_local::{SyncStore, TeamAuthSession, TeamAuthStore};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Deserialize)]
pub struct CreateTeamWorkspaceRequest {
    pub server_url: Option<String>,
    pub workspace_id: Option<String>,
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkspaceDetailResponse {
    pub workspace: Workspace,
    pub members: Vec<WorkspaceMember>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMemberRoleRequest {
    pub role: WorkspaceRole,
}

#[derive(Debug, Deserialize)]
pub struct CreateInvitationRequest {
    pub invitee_hint: String,
    pub role: WorkspaceRole,
    pub ttl_ms: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateInvitationResponse {
    pub invitation: WorkspaceInvitation,
    pub invite_token: String,
}

#[derive(Debug, Deserialize)]
pub struct AcceptInvitationRequest {
    pub server_url: String,
    pub invite_token: String,
    pub user_id: Option<String>,
    pub display_name: String,
}

#[derive(Debug, Serialize)]
pub struct AcceptInvitationLocalResponse {
    pub member: WorkspaceMember,
    pub server_url: String,
    pub token_saved: bool,
}

#[derive(Debug, Deserialize)]
struct ServerAcceptInvitationResponse {
    member: WorkspaceMember,
    bearer_token: String,
}

pub(crate) fn resolve_auth(
    state: &AppState,
    server_url: Option<&str>,
    workspace_id: Option<&WorkspaceId>,
) -> Result<TeamAuthSession, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let found = if let Some(url) = server_url.map(str::trim).filter(|s| !s.is_empty()) {
        TeamAuthStore::get_by_server(&conn, url).map_err(storage_error_to_http)?
    } else {
        TeamAuthStore::resolve_for_workspace(&conn, workspace_id).map_err(storage_error_to_http)?
    };
    found.ok_or_else(|| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "team_auth_required",
            "No team login session configured; run `runlark team login` first",
        )
    })
}

/// Creates a workspace on `runlark-server` and caches its initial snapshot locally.
///
/// # Errors
/// Returns `503` if offline/unauthenticated, or forwards server status.
pub async fn create_workspace(
    State(state): State<AppState>,
    Json(req): Json<CreateTeamWorkspaceRequest>,
) -> Result<(StatusCode, Json<WorkspaceDetailResponse>), ApiError> {
    let auth = resolve_auth(&state, req.server_url.as_deref(), None)?;
    let url = format!("{}/api/v1/workspaces", auth.server_url);
    let body = json!({
        "workspace_id": req.workspace_id,
        "name": req.name,
        "slug": req.slug,
    });
    let resp = TeamSyncWorker::new(state.clone())
        .http()
        .post(&url)
        .bearer_auth(&auth.bearer_token)
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "team_server_offline",
                e.to_string(),
            )
        })?;
    let detail: WorkspaceDetailResponse = parse_server_json_response(resp)
        .await
        .map_err(team_server_error_to_http)?;

    let mut updated_auth = auth.clone();
    if updated_auth.default_workspace_id.is_none() {
        updated_auth.default_workspace_id = Some(detail.workspace.id.clone());
        updated_auth.updated_at = Timestamp(AppState::current_timestamp_ms());
        if let Ok(conn) = state.get_connection()
            && let Err(err) = TeamAuthStore::save_session(&conn, &updated_auth)
        {
            tracing::warn!("Failed to update default workspace after create: {err}");
        }
    }
    if let Err(err) = TeamSyncWorker::new(state.clone())
        .sync_workspace(&auth.server_url, &auth.bearer_token, &detail.workspace.id)
        .await
    {
        tracing::warn!(
            workspace_id = %detail.workspace.id,
            "Post-create workspace sync deferred: {err}"
        );
        state.notify_team_sync();
    }
    Ok((StatusCode::CREATED, Json(detail)))
}

/// Fetches workspace metadata and members (from server when online, falling back to cache).
///
/// # Errors
/// Returns `404` if unknown, `503` if unauthenticated.
pub async fn get_workspace(
    State(state): State<AppState>,
    Path(wid): Path<String>,
) -> Result<Json<WorkspaceDetailResponse>, ApiError> {
    let wid_seg =
        validate_team_path_segment(&wid, "workspace_id").map_err(team_server_error_to_http)?;
    let ws_id = WorkspaceId::from(wid_seg);
    let auth = resolve_auth(&state, None, Some(&ws_id))?;
    let url = format!("{}/api/v1/workspaces/{wid_seg}", auth.server_url);
    let worker = TeamSyncWorker::new(state.clone());
    if let Ok(resp) = worker
        .http()
        .get(&url)
        .bearer_auth(&auth.bearer_token)
        .send()
        .await
    {
        let detail: WorkspaceDetailResponse = parse_server_json_response(resp)
            .await
            .map_err(team_server_error_to_http)?;
        Ok(Json(detail))
    } else {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        let ws = SyncStore::list_workspaces(&conn)
            .map_err(storage_error_to_http)?
            .into_iter()
            .find(|w| w.workspace.id == ws_id)
            .ok_or_else(|| {
                api_error(
                    StatusCode::NOT_FOUND,
                    "workspace_not_found",
                    format!("Workspace not found: {ws_id}"),
                )
            })?;
        let members =
            SyncStore::list_members_for_workspace(&conn, &ws_id).map_err(storage_error_to_http)?;
        Ok(Json(WorkspaceDetailResponse {
            workspace: ws.workspace,
            members,
        }))
    }
}

/// Lists workspace members (refreshing from server when online, falling back to local cache).
///
/// # Errors
/// Returns `503` if unauthenticated or `403`/`404` from server.
pub async fn list_members(
    State(state): State<AppState>,
    Path(wid): Path<String>,
) -> Result<Json<Vec<WorkspaceMember>>, ApiError> {
    let wid_seg =
        validate_team_path_segment(&wid, "workspace_id").map_err(team_server_error_to_http)?;
    let ws_id = WorkspaceId::from(wid_seg);
    let auth = resolve_auth(&state, None, Some(&ws_id))?;
    let url = format!("{}/api/v1/workspaces/{wid_seg}/members", auth.server_url);
    let worker = TeamSyncWorker::new(state.clone());
    if let Ok(resp) = worker
        .http()
        .get(&url)
        .bearer_auth(&auth.bearer_token)
        .send()
        .await
    {
        let members: Vec<WorkspaceMember> = parse_server_json_response(resp)
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
        SyncStore::ensure_workspace_placeholder(&conn, &auth.server_url, &ws_id, now)
            .map_err(storage_error_to_http)?;
        for m in &members {
            SyncStore::upsert_member(&conn, m).map_err(storage_error_to_http)?;
        }
        Ok(Json(members))
    } else {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        let members =
            SyncStore::list_members_for_workspace(&conn, &ws_id).map_err(storage_error_to_http)?;
        Ok(Json(members))
    }
}

/// Updates a workspace member's role via `runlark-server`.
///
/// # Errors
/// Returns `503` if offline or forwards server RBAC/validation status.
pub async fn update_member_role(
    State(state): State<AppState>,
    Path((wid, uid)): Path<(String, String)>,
    Json(req): Json<UpdateMemberRoleRequest>,
) -> Result<Json<WorkspaceMember>, ApiError> {
    let wid_seg =
        validate_team_path_segment(&wid, "workspace_id").map_err(team_server_error_to_http)?;
    let uid_seg = validate_team_path_segment(&uid, "user_id").map_err(team_server_error_to_http)?;
    let ws_id = WorkspaceId::from(wid_seg);
    let auth = resolve_auth(&state, None, Some(&ws_id))?;
    let url = format!(
        "{}/api/v1/workspaces/{wid_seg}/members/{uid_seg}",
        auth.server_url
    );
    let resp = TeamSyncWorker::new(state)
        .http()
        .patch(&url)
        .bearer_auth(&auth.bearer_token)
        .json(&json!({ "role": req.role }))
        .send()
        .await
        .map_err(|e| {
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "team_server_offline",
                e.to_string(),
            )
        })?;
    let member: WorkspaceMember = parse_server_json_response(resp)
        .await
        .map_err(team_server_error_to_http)?;
    Ok(Json(member))
}

/// Creates a workspace invitation on `runlark-server`.
///
/// # Errors
/// Returns `503` if offline or forwards server RBAC/validation status.
pub async fn create_invitation(
    State(state): State<AppState>,
    Path(wid): Path<String>,
    Json(req): Json<CreateInvitationRequest>,
) -> Result<(StatusCode, Json<CreateInvitationResponse>), ApiError> {
    let wid_seg =
        validate_team_path_segment(&wid, "workspace_id").map_err(team_server_error_to_http)?;
    let ws_id = WorkspaceId::from(wid_seg);
    let auth = resolve_auth(&state, None, Some(&ws_id))?;
    let url = format!(
        "{}/api/v1/workspaces/{wid_seg}/invitations",
        auth.server_url
    );
    let resp = TeamSyncWorker::new(state)
        .http()
        .post(&url)
        .bearer_auth(&auth.bearer_token)
        .json(&json!({
            "invitee_hint": req.invitee_hint,
            "role": req.role,
            "ttl_ms": req.ttl_ms,
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
    let created: CreateInvitationResponse = parse_server_json_response(resp)
        .await
        .map_err(team_server_error_to_http)?;
    Ok((StatusCode::CREATED, Json(created)))
}

/// Revokes a pending workspace invitation.
///
/// # Errors
/// Returns `503` if offline or forwards server status.
pub async fn revoke_invitation(
    State(state): State<AppState>,
    Path((wid, invite_id)): Path<(String, String)>,
) -> Result<Json<WorkspaceInvitation>, ApiError> {
    let wid_seg =
        validate_team_path_segment(&wid, "workspace_id").map_err(team_server_error_to_http)?;
    let inv_seg =
        validate_team_path_segment(&invite_id, "invite_id").map_err(team_server_error_to_http)?;
    let ws_id = WorkspaceId::from(wid_seg);
    let auth = resolve_auth(&state, None, Some(&ws_id))?;
    let url = format!(
        "{}/api/v1/workspaces/{wid_seg}/invitations/{inv_seg}/revoke",
        auth.server_url
    );
    let resp = TeamSyncWorker::new(state)
        .http()
        .post(&url)
        .bearer_auth(&auth.bearer_token)
        .send()
        .await
        .map_err(|e| {
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "team_server_offline",
                e.to_string(),
            )
        })?;
    let inv: WorkspaceInvitation = parse_server_json_response(resp)
        .await
        .map_err(team_server_error_to_http)?;
    Ok(Json(inv))
}

/// Accepts an invitation token against `server_url`, saves the returned session token locally,
/// and pulls the initial workspace snapshot into `SQLite`.
///
/// # Errors
/// Returns `503` if server unreachable or forwards server errors.
pub async fn accept_invitation(
    State(state): State<AppState>,
    Json(req): Json<AcceptInvitationRequest>,
) -> Result<Json<AcceptInvitationLocalResponse>, ApiError> {
    let clean_url = req.server_url.trim().trim_end_matches('/').to_string();
    if clean_url.is_empty()
        || req.invite_token.trim().is_empty()
        || req.display_name.trim().is_empty()
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_invite_accept",
            "server_url, invite_token, and display_name are required",
        ));
    }
    let worker = TeamSyncWorker::new(state.clone());
    let existing_token = {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        TeamAuthStore::get_by_server(&conn, &clean_url)
            .map_err(storage_error_to_http)?
            .map(|s| s.bearer_token)
    };
    let mut builder = worker
        .http()
        .post(format!("{clean_url}/api/v1/invitations/accept"))
        .json(&json!({
            "invite_token": req.invite_token.trim(),
            "user_id": req.user_id,
            "display_name": req.display_name.trim(),
        }));
    if let Some(tok) = existing_token {
        builder = builder.bearer_auth(tok);
    }
    let resp = builder.send().await.map_err(|e| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "team_server_offline",
            e.to_string(),
        )
    })?;
    let accepted: ServerAcceptInvitationResponse = parse_server_json_response(resp)
        .await
        .map_err(team_server_error_to_http)?;

    let now = Timestamp(AppState::current_timestamp_ms());
    let session = TeamAuthSession {
        server_url: clean_url.clone(),
        user_id: accepted.member.user_id.clone(),
        display_name: accepted.member.display_name.clone(),
        bearer_token: accepted.bearer_token.clone(),
        default_workspace_id: Some(accepted.member.workspace_id.clone()),
        runner_id: None,
        runner_name: None,
        updated_at: now,
    };
    {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        TeamAuthStore::save_session(&conn, &session).map_err(storage_error_to_http)?;
    }

    // The server already consumed the one-time invite token and issued a fresh bearer token.
    // Never delete the persisted token if follow-up snapshot sync transiently fails; defer to background sync.
    if let Err(err) = worker
        .sync_workspace(
            &clean_url,
            &accepted.bearer_token,
            &accepted.member.workspace_id,
        )
        .await
    {
        tracing::warn!(
            workspace_id = %accepted.member.workspace_id,
            "Initial workspace sync after accept_invitation deferred (session token preserved): {err}"
        );
        state.notify_team_sync();
    }

    Ok(Json(AcceptInvitationLocalResponse {
        member: accepted.member,
        server_url: clean_url,
        token_saved: true,
    }))
}
