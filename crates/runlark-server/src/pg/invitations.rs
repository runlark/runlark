use crate::auth::{AuthenticatedUser, authorize_member, generate_bearer_token, hash_secret_token};
use crate::error::ServerError;
use crate::pg::codec::row_to_invitation;
use crate::pg::workspaces::IssuedToken;
use crate::pg::{get_client, lock_workspace_write};
use deadpool_postgres::Pool;
use runlark_domain::{
    InvitationStatus, InviteId, Timestamp, UserId, WorkspaceAction, WorkspaceId,
    WorkspaceInvitation, WorkspaceMember, WorkspaceRole, WorkspaceSyncEvent,
};
use uuid::Uuid;

/// Creates a workspace invitation and returns `(WorkspaceInvitation, raw_invite_token)`.
///
/// # Errors
/// Returns [`ServerError::Forbidden`] if actor lacks `InviteMembers`, or [`ServerError::BadRequest`]
/// if trying to invite as `Owner` directly.
pub async fn create_invitation(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
    invitee_hint: &str,
    role: WorkspaceRole,
    ttl_ms: u64,
    now: Timestamp,
) -> Result<(WorkspaceInvitation, String), ServerError> {
    if role == WorkspaceRole::Owner {
        return Err(ServerError::BadRequest(
            "invitations cannot directly grant Owner role".to_string(),
        ));
    }
    if invitee_hint.trim().is_empty() {
        return Err(ServerError::BadRequest(
            "invitee_hint must not be empty".to_string(),
        ));
    }

    let mut client = get_client(pool).await?;
    let tx = client.transaction().await?;
    lock_workspace_write(&tx, workspace_id).await?;
    authorize_member(
        &tx,
        workspace_id,
        &actor.user_id,
        WorkspaceAction::InviteMembers,
    )
    .await?;

    let invite_id = InviteId::from(format!("inv_{}", Uuid::new_v4().simple()));
    let raw_invite_token = format!("inv_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let token_hash = hash_secret_token(&raw_invite_token);
    let expires_at = Timestamp(now.0.saturating_add(ttl_ms.max(60_000)));

    tx.execute(
        concat!(
            "INSERT INTO workspace_invitations ",
            "(invite_id, workspace_id, invitee_hint, role, token_hash, status, invited_by, expires_at, created_at) ",
            "VALUES ($1, $2, $3, $4, $5, 'pending', $6, $7, $8)",
        ),
        &[
            &invite_id.as_ref(),
            &workspace_id.as_ref(),
            &invitee_hint,
            &role.as_str(),
            &token_hash,
            &actor.user_id.as_ref(),
            &ServerError::to_pg_i64(expires_at.0)?,
            &ServerError::to_pg_i64(now.0)?,
        ],
    )
    .await?;
    tx.commit().await?;

    Ok((
        WorkspaceInvitation {
            id: invite_id,
            workspace_id: workspace_id.clone(),
            invitee_hint: invitee_hint.to_string(),
            role,
            status: InvitationStatus::Pending,
            invited_by: actor.user_id.clone(),
            expires_at,
            created_at: now,
            accepted_at: None,
            accepted_by: None,
        },
        raw_invite_token,
    ))
}

/// Revokes a pending invitation.
///
/// # Errors
/// Returns [`ServerError::NotFound`] or [`ServerError::Conflict`] if already accepted/revoked.
pub async fn revoke_invitation(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
    invite_id: &InviteId,
) -> Result<WorkspaceInvitation, ServerError> {
    let mut client = get_client(pool).await?;
    let tx = client.transaction().await?;
    lock_workspace_write(&tx, workspace_id).await?;
    authorize_member(
        &tx,
        workspace_id,
        &actor.user_id,
        WorkspaceAction::InviteMembers,
    )
    .await?;

    let row = tx
        .query_opt(
            concat!(
                "UPDATE workspace_invitations ",
                "SET status = 'revoked' ",
                "WHERE invite_id = $1 AND workspace_id = $2 AND status = 'pending' ",
                "RETURNING invite_id, workspace_id, invitee_hint, role, status, invited_by, expires_at, created_at, accepted_at, accepted_by",
            ),
            &[&invite_id.as_ref(), &workspace_id.as_ref()],
        )
        .await?
        .ok_or_else(|| {
            ServerError::conflict(
                "invitation_not_pending",
                format!("invitation {invite_id} is missing or not pending"),
                None,
            )
        })?;
    tx.commit().await?;

    row_to_invitation(&row)
}

/// Lists workspace invitations (newest first) without exposing secret token hashes.
///
/// # Errors
/// Returns [`ServerError::Forbidden`] if actor lacks `InviteMembers` permission.
pub async fn list_invitations(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
) -> Result<Vec<WorkspaceInvitation>, ServerError> {
    let client = get_client(pool).await?;
    authorize_member(
        &client,
        workspace_id,
        &actor.user_id,
        WorkspaceAction::InviteMembers,
    )
    .await?;

    let rows = client
        .query(
            concat!(
                "SELECT invite_id, workspace_id, invitee_hint, role, status, invited_by, ",
                "expires_at, created_at, accepted_at, accepted_by ",
                "FROM workspace_invitations ",
                "WHERE workspace_id = $1 ",
                "ORDER BY created_at DESC, invite_id ASC",
            ),
            &[&workspace_id.as_ref()],
        )
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_invitation(&row)?);
    }
    Ok(out)
}

/// Accepts an invitation token, enrolling the caller in `workspace_members` and issuing a token.
///
/// Unauthenticated callers cannot claim an existing `user_id` (`401 Unauthorized`); authenticated
/// callers may only link their own `authenticated_caller.user_id` (`403 Forbidden` on mismatch).
/// Existing workspace members cannot have their role overwritten via invitation (`409 Conflict`).
///
/// # Errors
/// Returns [`ServerError`] if invitation is unknown/expired/revoked, or user claims conflict.
pub async fn accept_invitation(
    pool: &Pool,
    authenticated_caller: Option<&AuthenticatedUser>,
    raw_invite_token: &str,
    requested_user_id: Option<UserId>,
    display_name: &str,
    now: Timestamp,
) -> Result<(WorkspaceMember, IssuedToken), ServerError> {
    if display_name.trim().is_empty() {
        return Err(ServerError::BadRequest(
            "display_name must not be empty".to_string(),
        ));
    }

    let resolved_uid = resolve_invitation_user_id(authenticated_caller, requested_user_id)?;
    let token_hash = hash_secret_token(raw_invite_token);
    let mut client = get_client(pool).await?;
    let tx = client.transaction().await?;
    let now_i64 = ServerError::to_pg_i64(now.0)?;

    let ws_row = tx
        .query_opt(
            "SELECT workspace_id FROM workspace_invitations WHERE token_hash = $1",
            &[&token_hash],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound("invitation token not found".to_string()))?;
    let workspace_id = WorkspaceId::from(ws_row.get::<_, String>("workspace_id"));

    // Match revoke_invitation's lock order: workspace advisory lock before invitation row lock.
    lock_workspace_write(&tx, &workspace_id).await?;

    let row = tx
        .query_opt(
            concat!(
                "SELECT invite_id, workspace_id, invitee_hint, role, status, invited_by, expires_at, created_at, accepted_at, accepted_by ",
                "FROM workspace_invitations ",
                "WHERE token_hash = $1 ",
                "FOR UPDATE",
            ),
            &[&token_hash],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound("invitation token not found".to_string()))?;

    let inv = row_to_invitation(&row)?;
    if inv.status != InvitationStatus::Pending {
        return Err(ServerError::conflict(
            "invitation_not_pending",
            format!("invitation {} is {:?}", inv.id, inv.status),
            None,
        ));
    }
    if inv.expires_at.0 <= now.0 {
        tx.execute(
            "UPDATE workspace_invitations SET status = 'expired' WHERE invite_id = $1",
            &[&inv.id.as_ref()],
        )
        .await?;
        tx.commit().await?;
        return Err(ServerError::conflict(
            "invitation_expired",
            "invitation token has expired",
            None,
        ));
    }

    let raw_bearer = finalize_invitation_acceptance(
        &tx,
        authenticated_caller,
        &inv,
        &resolved_uid,
        display_name,
        now,
        now_i64,
    )
    .await?;
    tx.commit().await?;

    let effective_name =
        authenticated_caller.map_or_else(|| display_name.to_string(), |u| u.display_name.clone());
    let member = WorkspaceMember {
        workspace_id: inv.workspace_id,
        user_id: resolved_uid.clone(),
        display_name: effective_name.clone(),
        role: inv.role,
        joined_at: now,
    };
    Ok((
        member,
        IssuedToken {
            user_id: resolved_uid,
            display_name: effective_name,
            raw_token: raw_bearer,
        },
    ))
}

fn resolve_invitation_user_id(
    authenticated_caller: Option<&AuthenticatedUser>,
    requested_user_id: Option<UserId>,
) -> Result<UserId, ServerError> {
    if let Some(caller) = authenticated_caller {
        if let Some(ref req_uid) = requested_user_id
            && req_uid != &caller.user_id
        {
            return Err(ServerError::Forbidden(
                "cannot accept invitation on behalf of another user_id".to_string(),
            ));
        }
        return Ok(caller.user_id.clone());
    }
    Ok(requested_user_id
        .unwrap_or_else(|| UserId::from(format!("usr_{}", Uuid::new_v4().simple()))))
}

async fn finalize_invitation_acceptance(
    tx: &deadpool_postgres::Transaction<'_>,
    authenticated_caller: Option<&AuthenticatedUser>,
    inv: &WorkspaceInvitation,
    resolved_uid: &UserId,
    display_name: &str,
    now: Timestamp,
    now_i64: i64,
) -> Result<String, ServerError> {
    let existing_user = tx
        .query_opt(
            "SELECT display_name FROM users WHERE user_id = $1 FOR UPDATE",
            &[&resolved_uid.as_ref()],
        )
        .await?;

    if authenticated_caller.is_none() && existing_user.is_some() {
        return Err(ServerError::Unauthorized(
            "cannot claim an existing user_id without authentication".to_string(),
        ));
    }

    let effective_display_name = if let Some(row) = existing_user {
        row.get::<_, String>("display_name")
    } else {
        tx.execute(
            concat!(
                "INSERT INTO users (user_id, display_name, email, created_at) ",
                "VALUES ($1, $2, NULL, $3)",
            ),
            &[&resolved_uid.as_ref(), &display_name, &now_i64],
        )
        .await?;
        display_name.to_string()
    };

    let existing_member = tx
        .query_opt(
            "SELECT 1 FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
            &[&inv.workspace_id.as_ref(), &resolved_uid.as_ref()],
        )
        .await?;
    if existing_member.is_some() {
        return Err(ServerError::conflict(
            "already_workspace_member",
            format!(
                "user {resolved_uid} is already a member of workspace {}",
                inv.workspace_id
            ),
            None,
        ));
    }

    let (raw_bearer, bearer_hash) = generate_bearer_token();
    tx.execute(
        concat!(
            "INSERT INTO auth_tokens (token_hash, user_id, label, expires_at, revoked_at, created_at) ",
            "VALUES ($1, $2, 'invite-accept', NULL, NULL, $3)",
        ),
        &[&bearer_hash, &resolved_uid.as_ref(), &now_i64],
    )
    .await?;

    tx.execute(
        concat!(
            "INSERT INTO workspace_members (workspace_id, user_id, role, joined_at) ",
            "VALUES ($1, $2, $3, $4)",
        ),
        &[
            &inv.workspace_id.as_ref(),
            &resolved_uid.as_ref(),
            &inv.role.as_str(),
            &now_i64,
        ],
    )
    .await?;

    tx.execute(
        concat!(
            "UPDATE workspace_invitations ",
            "SET status = 'accepted', accepted_at = $2, accepted_by = $3 ",
            "WHERE invite_id = $1",
        ),
        &[&inv.id.as_ref(), &now_i64, &resolved_uid.as_ref()],
    )
    .await?;

    let member = WorkspaceMember {
        workspace_id: inv.workspace_id.clone(),
        user_id: resolved_uid.clone(),
        display_name: effective_display_name,
        role: inv.role,
        joined_at: now,
    };
    let ev_json = serde_json::to_string(&WorkspaceSyncEvent::MemberUpdated { member })?;
    tx.execute(
        concat!(
            "INSERT INTO workspace_events ",
            "(workspace_id, project_id, aggregate_type, aggregate_id, event_type, payload_json, actor_user_id, created_at) ",
            "VALUES ($1, NULL, 'member', $2, 'member_updated', $3, $4, $5)",
        ),
        &[
            &inv.workspace_id.as_ref(),
            &resolved_uid.as_ref(),
            &ev_json,
            &resolved_uid.as_ref(),
            &now_i64,
        ],
    )
    .await?;

    Ok(raw_bearer)
}
