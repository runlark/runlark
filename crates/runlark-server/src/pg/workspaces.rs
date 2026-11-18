use crate::auth::{AuthenticatedUser, authorize_member, generate_bearer_token};
use crate::error::ServerError;
use crate::pg::codec::row_to_member;
use crate::pg::{get_client, lock_workspace_write};
use deadpool_postgres::Pool;
use runlark_domain::{
    Revision, Timestamp, UserId, Workspace, WorkspaceAction, WorkspaceId, WorkspaceMember,
    WorkspaceRole, WorkspaceSyncEvent,
};
use uuid::Uuid;

/// Payload returned when bootstrapping a user or accepting a workspace invitation.
#[derive(Debug, Clone)]
pub struct IssuedToken {
    pub user_id: UserId,
    pub display_name: String,
    pub raw_token: String,
}

/// Optional workspace parameters for atomic [`bootstrap_owner`] provisioning.
#[derive(Debug, Clone)]
pub struct BootstrapWorkspaceSpec {
    pub workspace_id: Option<WorkspaceId>,
    pub name: String,
    pub slug: String,
}

/// Atomically provisions an initial user + bearer token and (optionally) a workspace in one
/// `PostgreSQL` transaction so workspace errors never leave orphaned users or tokens.
///
/// # Errors
/// Returns [`ServerError`] if user/token insertion or workspace creation fails.
pub async fn bootstrap_owner(
    pool: &Pool,
    user_id: &UserId,
    display_name: &str,
    email: Option<&str>,
    label: &str,
    workspace: Option<BootstrapWorkspaceSpec>,
    now: Timestamp,
) -> Result<(IssuedToken, Option<WorkspaceId>), ServerError> {
    if display_name.trim().is_empty() {
        return Err(ServerError::BadRequest(
            "display_name must not be empty".to_string(),
        ));
    }
    let mut client = get_client(pool).await?;
    let tx = client.transaction().await?;
    let issued = issue_user_token_in_tx(&tx, user_id, display_name, email, label, now).await?;
    let created_ws_id = if let Some(ws) = workspace {
        let actor = AuthenticatedUser {
            user_id: issued.user_id.clone(),
            display_name: issued.display_name.clone(),
        };
        let (created, _) =
            create_workspace_in_tx(&tx, &actor, ws.workspace_id, &ws.name, &ws.slug, now).await?;
        Some(created.id)
    } else {
        None
    };
    tx.commit().await?;
    Ok((issued, created_ws_id))
}

/// Inserts or updates a user and issues a fresh bearer token.
///
/// # Errors
/// Returns [`ServerError`] if `PostgreSQL` insertion fails.
pub async fn issue_user_token(
    pool: &Pool,
    user_id: &UserId,
    display_name: &str,
    email: Option<&str>,
    label: &str,
    now: Timestamp,
) -> Result<IssuedToken, ServerError> {
    let mut client = get_client(pool).await?;
    let tx = client.transaction().await?;
    let issued = issue_user_token_in_tx(&tx, user_id, display_name, email, label, now).await?;
    tx.commit().await?;
    Ok(issued)
}

async fn issue_user_token_in_tx(
    tx: &deadpool_postgres::Transaction<'_>,
    user_id: &UserId,
    display_name: &str,
    email: Option<&str>,
    label: &str,
    now: Timestamp,
) -> Result<IssuedToken, ServerError> {
    if user_id.0.trim().is_empty() {
        return Err(ServerError::BadRequest(
            "user_id must not be empty".to_string(),
        ));
    }
    if display_name.trim().is_empty() {
        return Err(ServerError::BadRequest(
            "display_name must not be empty".to_string(),
        ));
    }
    let now_i64 = ServerError::to_pg_i64(now.0)?;

    tx.execute(
        concat!(
            "INSERT INTO users (user_id, display_name, email, created_at) ",
            "VALUES ($1, $2, $3, $4) ",
            "ON CONFLICT (user_id) DO UPDATE SET display_name = EXCLUDED.display_name",
        ),
        &[&user_id.as_ref(), &display_name, &email, &now_i64],
    )
    .await?;

    let (raw_token, token_hash) = generate_bearer_token();
    tx.execute(
        concat!(
            "INSERT INTO auth_tokens (token_hash, user_id, label, expires_at, revoked_at, created_at) ",
            "VALUES ($1, $2, $3, NULL, NULL, $4)",
        ),
        &[&token_hash, &user_id.as_ref(), &label, &now_i64],
    )
    .await?;

    Ok(IssuedToken {
        user_id: user_id.clone(),
        display_name: display_name.to_string(),
        raw_token,
    })
}

/// Creates a new shared workspace and enrolls the creator as `Owner`.
///
/// # Errors
/// Returns [`ServerError::Conflict`] if `slug` already exists, or [`ServerError`] on DB failure.
pub async fn create_workspace(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: Option<WorkspaceId>,
    name: &str,
    slug: &str,
    now: Timestamp,
) -> Result<(Workspace, WorkspaceMember), ServerError> {
    let mut client = get_client(pool).await?;
    let tx = client.transaction().await?;
    let res = create_workspace_in_tx(&tx, actor, workspace_id, name, slug, now).await?;
    tx.commit().await?;
    Ok(res)
}

async fn create_workspace_in_tx(
    tx: &deadpool_postgres::Transaction<'_>,
    actor: &AuthenticatedUser,
    workspace_id: Option<WorkspaceId>,
    name: &str,
    slug: &str,
    now: Timestamp,
) -> Result<(Workspace, WorkspaceMember), ServerError> {
    if let Some(ref id) = workspace_id
        && id.0.trim().is_empty()
    {
        return Err(ServerError::BadRequest(
            "workspace_id must not be empty".to_string(),
        ));
    }
    if name.trim().is_empty() || slug.trim().is_empty() {
        return Err(ServerError::BadRequest(
            "workspace name and slug must not be empty".to_string(),
        ));
    }

    let wid = workspace_id
        .unwrap_or_else(|| WorkspaceId::from(format!("ws_{}", Uuid::new_v4().simple())));
    lock_workspace_write(tx, &wid).await?;
    let now_i64 = ServerError::to_pg_i64(now.0)?;

    let existing_slug = tx
        .query_opt(
            "SELECT workspace_id FROM workspaces WHERE slug = $1",
            &[&slug],
        )
        .await?;
    if existing_slug.is_some() {
        return Err(ServerError::conflict(
            "workspace_slug_exists",
            format!("workspace slug already exists: {slug}"),
            None,
        ));
    }

    tx.execute(
        concat!(
            "INSERT INTO workspaces (workspace_id, name, slug, created_by, revision, created_at, updated_at) ",
            "VALUES ($1, $2, $3, $4, 1, $5, $5)",
        ),
        &[
            &wid.as_ref(),
            &name,
            &slug,
            &actor.user_id.as_ref(),
            &now_i64,
        ],
    )
    .await?;

    tx.execute(
        concat!(
            "INSERT INTO workspace_members (workspace_id, user_id, role, joined_at) ",
            "VALUES ($1, $2, $3, $4)",
        ),
        &[
            &wid.as_ref(),
            &actor.user_id.as_ref(),
            &WorkspaceRole::Owner.as_str(),
            &now_i64,
        ],
    )
    .await?;

    let member = WorkspaceMember {
        workspace_id: wid.clone(),
        user_id: actor.user_id.clone(),
        display_name: actor.display_name.clone(),
        role: WorkspaceRole::Owner,
        joined_at: now,
    };

    let ev_json = serde_json::to_string(&WorkspaceSyncEvent::MemberUpdated {
        member: member.clone(),
    })?;
    tx.execute(
        concat!(
            "INSERT INTO workspace_events ",
            "(workspace_id, project_id, aggregate_type, aggregate_id, event_type, payload_json, actor_user_id, created_at) ",
            "VALUES ($1, NULL, 'member', $2, 'member_updated', $3, $4, $5)",
        ),
        &[
            &wid.as_ref(),
            &actor.user_id.as_ref(),
            &ev_json,
            &actor.user_id.as_ref(),
            &now_i64,
        ],
    )
    .await?;

    let workspace = Workspace {
        id: wid,
        name: name.to_string(),
        slug: slug.to_string(),
        created_by: actor.user_id.clone(),
        revision: Revision(1),
        created_at: now,
        updated_at: now,
    };
    Ok((workspace, member))
}

/// Fetches a workspace and its members after checking current read membership.
///
/// # Errors
/// Returns [`ServerError::Forbidden`] if caller is not a member, or [`ServerError::NotFound`].
pub async fn get_workspace_with_members(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
) -> Result<(Workspace, Vec<WorkspaceMember>), ServerError> {
    let client = get_client(pool).await?;
    authorize_member(
        &client,
        workspace_id,
        &actor.user_id,
        WorkspaceAction::ReadWorkspace,
    )
    .await?;

    let ws_row = client
        .query_opt(
            concat!(
                "SELECT workspace_id, name, slug, created_by, revision, created_at, updated_at ",
                "FROM workspaces WHERE workspace_id = $1",
            ),
            &[&workspace_id.as_ref()],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound(format!("workspace not found: {workspace_id}")))?;

    let workspace = Workspace {
        id: WorkspaceId::from(ws_row.get::<_, String>("workspace_id")),
        name: ws_row.get("name"),
        slug: ws_row.get("slug"),
        created_by: UserId::from(ws_row.get::<_, String>("created_by")),
        revision: Revision(ServerError::from_pg_i64(
            ws_row.get("revision"),
            "revision",
        )?),
        created_at: Timestamp(ServerError::from_pg_i64(
            ws_row.get("created_at"),
            "created_at",
        )?),
        updated_at: Timestamp(ServerError::from_pg_i64(
            ws_row.get("updated_at"),
            "updated_at",
        )?),
    };

    let rows = client
        .query(
            concat!(
                "SELECT wm.workspace_id, wm.user_id, u.display_name, wm.role, wm.joined_at ",
                "FROM workspace_members wm ",
                "INNER JOIN users u ON u.user_id = wm.user_id ",
                "WHERE wm.workspace_id = $1 ",
                "ORDER BY wm.joined_at ASC, wm.user_id ASC",
            ),
            &[&workspace_id.as_ref()],
        )
        .await?;

    let mut members = Vec::with_capacity(rows.len());
    for row in &rows {
        members.push(row_to_member(row)?);
    }

    Ok((workspace, members))
}

/// Updates an existing member's role, ensuring hierarchical RBAC (`Maintainer` cannot grant or
/// modify `Owner`) and that at least one `Owner` remains in the workspace.
///
/// # Errors
/// Returns [`ServerError::Forbidden`] if actor lacks `ManageMembers` or attempts to grant/modify
/// `Owner` without being an `Owner`, [`ServerError::Conflict`] if demoting the last owner, or
/// [`ServerError::NotFound`] if target is not a member.
pub async fn update_member_role(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
    target_user_id: &UserId,
    new_role: WorkspaceRole,
    now: Timestamp,
) -> Result<WorkspaceMember, ServerError> {
    let mut client = get_client(pool).await?;
    let tx = client.transaction().await?;
    lock_workspace_write(&tx, workspace_id).await?;
    let actor_role = authorize_member(
        &tx,
        workspace_id,
        &actor.user_id,
        WorkspaceAction::ManageMembers,
    )
    .await?;

    if actor_role != WorkspaceRole::Owner && new_role == WorkspaceRole::Owner {
        return Err(ServerError::Forbidden(
            "only an Owner may grant the Owner role".to_string(),
        ));
    }

    let target_row = tx
        .query_opt(
            concat!(
                "SELECT wm.workspace_id, wm.user_id, u.display_name, wm.role, wm.joined_at ",
                "FROM workspace_members wm ",
                "INNER JOIN users u ON u.user_id = wm.user_id ",
                "WHERE wm.workspace_id = $1 AND wm.user_id = $2 ",
                "FOR UPDATE",
            ),
            &[&workspace_id.as_ref(), &target_user_id.as_ref()],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound(format!("member not found: {target_user_id}")))?;

    let current_member = row_to_member(&target_row)?;
    if current_member.role == WorkspaceRole::Owner && actor_role != WorkspaceRole::Owner {
        return Err(ServerError::Forbidden(
            "only an Owner may modify an existing Owner".to_string(),
        ));
    }
    if current_member.role == WorkspaceRole::Owner && new_role != WorkspaceRole::Owner {
        let owner_count: i64 = tx
            .query_one(
                "SELECT COUNT(*) FROM workspace_members WHERE workspace_id = $1 AND role = 'owner'",
                &[&workspace_id.as_ref()],
            )
            .await?
            .get(0);
        if owner_count <= 1 {
            return Err(ServerError::conflict(
                "last_owner_protected",
                "cannot demote the last workspace Owner",
                None,
            ));
        }
    }

    tx.execute(
        "UPDATE workspace_members SET role = $3 WHERE workspace_id = $1 AND user_id = $2",
        &[
            &workspace_id.as_ref(),
            &target_user_id.as_ref(),
            &new_role.as_str(),
        ],
    )
    .await?;

    let updated = WorkspaceMember {
        role: new_role,
        ..current_member
    };
    let ev_json = serde_json::to_string(&WorkspaceSyncEvent::MemberUpdated {
        member: updated.clone(),
    })?;
    let now_i64 = ServerError::to_pg_i64(now.0)?;
    tx.execute(
        concat!(
            "INSERT INTO workspace_events ",
            "(workspace_id, project_id, aggregate_type, aggregate_id, event_type, payload_json, actor_user_id, created_at) ",
            "VALUES ($1, NULL, 'member', $2, 'member_updated', $3, $4, $5)",
        ),
        &[
            &workspace_id.as_ref(),
            &target_user_id.as_ref(),
            &ev_json,
            &actor.user_id.as_ref(),
            &now_i64,
        ],
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}
