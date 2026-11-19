use crate::auth::{AuthenticatedUser, authenticate_headers};
use crate::config::ServerConfig;
use crate::error::ServerError;
use crate::pg::promote::{self, PromoteProjectPayload};
use crate::pg::{invitations, workspaces};
use crate::server::ServerState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use runlark_domain::{
    Decision, InviteId, Run, ServerPromotionAck, Task, TeamProjectSpec, Timestamp, UserId,
    Workspace, WorkspaceId, WorkspaceInvitation, WorkspaceMember, WorkspaceRole,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct WhoamiResponse {
    pub user_id: String,
    pub display_name: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateWorkspaceRequest {
    pub workspace_id: Option<String>,
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
pub struct CreateInvitationResponse {
    pub invitation: WorkspaceInvitation,
    pub invite_token: String,
}

#[derive(Debug, Deserialize)]
pub struct AcceptInvitationRequest {
    pub invite_token: String,
    pub user_id: Option<String>,
    pub display_name: String,
}

#[derive(Debug, Deserialize)]
pub struct AcceptInvitationPathRequest {
    pub user_id: Option<String>,
    pub display_name: String,
}

#[derive(Debug, Serialize)]
pub struct AcceptInvitationResponse {
    pub member: WorkspaceMember,
    pub bearer_token: String,
}

#[derive(Debug, Deserialize)]
pub struct PromoteProjectRequest {
    pub project: TeamProjectSpec,
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub runs: Vec<Run>,
    #[serde(default)]
    pub decisions: Vec<Decision>,
}

/// Returns the currently authenticated user identity.
///
/// # Errors
/// Returns [`ServerError::Unauthorized`] if bearer token is invalid or revoked.
pub async fn whoami(
    State(state): State<ServerState>,
    headers: HeaderMap,
) -> Result<Json<WhoamiResponse>, ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    Ok(Json(WhoamiResponse {
        user_id: user.user_id.0,
        display_name: user.display_name,
    }))
}

/// Creates a workspace and assigns caller as `Owner`.
///
/// # Errors
/// Returns [`ServerError`] on auth failure, invalid input, or duplicate slug.
pub async fn create_workspace(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(req): Json<CreateWorkspaceRequest>,
) -> Result<(StatusCode, Json<WorkspaceDetailResponse>), ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let now = Timestamp(ServerConfig::current_timestamp_ms());
    let (workspace, member) = workspaces::create_workspace(
        &state.pool,
        &user,
        req.workspace_id.map(WorkspaceId::from),
        &req.name,
        &req.slug,
        now,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(WorkspaceDetailResponse {
            workspace,
            members: vec![member],
        }),
    ))
}

/// Returns workspace metadata and current member roles.
///
/// # Errors
/// Returns [`ServerError`] on auth/membership denial or missing workspace.
pub async fn get_workspace(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(wid): Path<String>,
) -> Result<Json<WorkspaceDetailResponse>, ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let (workspace, members) =
        workspaces::get_workspace_with_members(&state.pool, &user, &WorkspaceId::from(wid)).await?;
    Ok(Json(WorkspaceDetailResponse { workspace, members }))
}

/// Lists current members and their roles in `wid`.
///
/// # Errors
/// Returns [`ServerError`] on auth/membership denial or missing workspace.
pub async fn list_members(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(wid): Path<String>,
) -> Result<Json<Vec<WorkspaceMember>>, ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let (_, members) =
        workspaces::get_workspace_with_members(&state.pool, &user, &WorkspaceId::from(wid)).await?;
    Ok(Json(members))
}

/// Updates a member's workspace role.
///
/// # Errors
/// Returns [`ServerError`] on permission denial, missing member, or last-owner protection.
pub async fn update_member_role(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path((wid, uid)): Path<(String, String)>,
    Json(req): Json<UpdateMemberRoleRequest>,
) -> Result<Json<WorkspaceMember>, ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let now = Timestamp(ServerConfig::current_timestamp_ms());
    let updated = workspaces::update_member_role(
        &state.pool,
        &user,
        &WorkspaceId::from(wid),
        &UserId::from(uid),
        req.role,
        now,
    )
    .await?;
    Ok(Json(updated))
}

/// Lists invitations for a workspace (without exposing secret tokens).
///
/// # Errors
/// Returns [`ServerError`] on auth/permission denial.
pub async fn list_invitations(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(wid): Path<String>,
) -> Result<Json<Vec<WorkspaceInvitation>>, ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let list = invitations::list_invitations(&state.pool, &user, &WorkspaceId::from(wid)).await?;
    Ok(Json(list))
}

/// Creates a workspace invitation returning the one-time secret `invite_token`.
///
/// # Errors
/// Returns [`ServerError`] on permission denial or invalid role.
pub async fn create_invitation(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(wid): Path<String>,
    Json(req): Json<CreateInvitationRequest>,
) -> Result<(StatusCode, Json<CreateInvitationResponse>), ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let now = Timestamp(ServerConfig::current_timestamp_ms());
    let ttl_ms = req.ttl_ms.unwrap_or(7 * 24 * 60 * 60 * 1000);
    let (invitation, invite_token) = invitations::create_invitation(
        &state.pool,
        &user,
        &WorkspaceId::from(wid),
        &req.invitee_hint,
        req.role,
        ttl_ms,
        now,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(CreateInvitationResponse {
            invitation,
            invite_token,
        }),
    ))
}

/// Revokes a pending invitation.
///
/// # Errors
/// Returns [`ServerError`] if caller lacks permission or invitation is not pending.
pub async fn revoke_invitation(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path((wid, invite_id)): Path<(String, String)>,
) -> Result<Json<WorkspaceInvitation>, ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let inv = invitations::revoke_invitation(
        &state.pool,
        &user,
        &WorkspaceId::from(wid),
        &InviteId::from(invite_id),
    )
    .await?;
    Ok(Json(inv))
}

async fn optional_authenticated_caller(
    state: &ServerState,
    headers: &HeaderMap,
) -> Result<Option<AuthenticatedUser>, ServerError> {
    if headers.contains_key(axum::http::header::AUTHORIZATION) {
        Ok(Some(authenticate_headers(&state.pool, headers).await?))
    } else {
        Ok(None)
    }
}

/// Accepts an invitation token and returns the new membership plus a bearer token.
///
/// # Errors
/// Returns [`ServerError`] if invitation token is unknown, expired, or already used, or if an
/// unauthenticated caller attempts to claim an existing `user_id`.
pub async fn accept_invitation(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(req): Json<AcceptInvitationRequest>,
) -> Result<Json<AcceptInvitationResponse>, ServerError> {
    let caller = optional_authenticated_caller(&state, &headers).await?;
    let now = Timestamp(ServerConfig::current_timestamp_ms());
    let (member, issued) = invitations::accept_invitation(
        &state.pool,
        caller.as_ref(),
        &req.invite_token,
        req.user_id.map(UserId::from),
        &req.display_name,
        now,
    )
    .await?;
    Ok(Json(AcceptInvitationResponse {
        member,
        bearer_token: issued.raw_token,
    }))
}

/// Accepts an invitation token passed in the URL path and returns membership + bearer token.
///
/// # Errors
/// Returns [`ServerError`] if invitation token is unknown, expired, or already used, or if an
/// unauthenticated caller attempts to claim an existing `user_id`.
pub async fn accept_invitation_by_path(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Json(req): Json<AcceptInvitationPathRequest>,
) -> Result<Json<AcceptInvitationResponse>, ServerError> {
    let caller = optional_authenticated_caller(&state, &headers).await?;
    let now = Timestamp(ServerConfig::current_timestamp_ms());
    let (member, issued) = invitations::accept_invitation(
        &state.pool,
        caller.as_ref(),
        &token,
        req.user_id.map(UserId::from),
        &req.display_name,
        now,
    )
    .await?;
    Ok(Json(AcceptInvitationResponse {
        member,
        bearer_token: issued.raw_token,
    }))
}

/// Promotes a solo project snapshot to team ownership, returning [`ServerPromotionAck`].
///
/// # Errors
/// Returns [`ServerError`] if active/queued runs exist, permission denied, or already promoted.
pub async fn promote_project(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(wid): Path<String>,
    Json(req): Json<PromoteProjectRequest>,
) -> Result<Json<ServerPromotionAck>, ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let now = Timestamp(ServerConfig::current_timestamp_ms());
    let payload = PromoteProjectPayload {
        project: req.project,
        tasks: req.tasks,
        runs: req.runs,
        decisions: req.decisions,
    };
    let ack =
        promote::promote_solo_project(&state.pool, &user, &WorkspaceId::from(wid), payload, now)
            .await?;
    Ok(Json(ack))
}
