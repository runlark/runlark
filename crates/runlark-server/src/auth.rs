use crate::cas::sha256_hex;
use crate::config::ServerConfig;
use crate::error::ServerError;
use crate::pg::get_client;
use axum::http::HeaderMap;
use deadpool_postgres::Pool;
use runlark_domain::{
    UserId, WorkspaceAction, WorkspaceId, WorkspaceRole, authorize_workspace_action,
};
use uuid::Uuid;

/// Authenticated caller identity resolved from a valid non-revoked bearer token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedUser {
    pub user_id: UserId,
    pub display_name: String,
}

/// Generates a fresh bearer token with prefix `rlk_` and returns `(raw_token, sha256_hash)`.
#[must_use]
pub fn generate_bearer_token() -> (String, String) {
    let raw = format!("rlk_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let hash = sha256_hex(raw.as_bytes());
    (raw, hash)
}

/// Hashes a raw bearer or invitation token with SHA-256.
#[must_use]
pub fn hash_secret_token(raw_token: &str) -> String {
    sha256_hex(raw_token.trim().as_bytes())
}

/// Extracts and authenticates a caller from `Authorization: Bearer <token>` against `PostgreSQL`.
///
/// Always queries current token revocation/expiry status from `PostgreSQL` rather than trusting
/// cached credentials.
///
/// # Errors
/// Returns [`ServerError::Unauthorized`] if the header is missing, malformed, revoked, or expired.
pub async fn authenticate_headers(
    pool: &Pool,
    headers: &HeaderMap,
) -> Result<AuthenticatedUser, ServerError> {
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ServerError::Unauthorized("missing Authorization header".to_string()))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| ServerError::Unauthorized("expected Bearer token".to_string()))?;

    let token_hash = hash_secret_token(token);
    let now = ServerError::to_pg_i64(ServerConfig::current_timestamp_ms())?;
    let client = get_client(pool).await?;

    let row = client
        .query_opt(
            concat!(
                "SELECT u.user_id, u.display_name, t.expires_at, t.revoked_at ",
                "FROM auth_tokens t ",
                "INNER JOIN users u ON u.user_id = t.user_id ",
                "WHERE t.token_hash = $1",
            ),
            &[&token_hash],
        )
        .await?
        .ok_or_else(|| ServerError::Unauthorized("invalid bearer token".to_string()))?;

    let revoked_at: Option<i64> = row.get("revoked_at");
    if revoked_at.is_some() {
        return Err(ServerError::Unauthorized(
            "bearer token revoked".to_string(),
        ));
    }

    let expires_at: Option<i64> = row.get("expires_at");
    if expires_at.is_some_and(|exp| exp <= now) {
        return Err(ServerError::Unauthorized(
            "bearer token expired".to_string(),
        ));
    }

    Ok(AuthenticatedUser {
        user_id: UserId::from(row.get::<_, String>("user_id")),
        display_name: row.get("display_name"),
    })
}

/// Loads the caller's current [`WorkspaceRole`] in `workspace_id` from `PostgreSQL` and enforces
/// [`authorize_workspace_action`].
///
/// # Errors
/// Returns [`ServerError::Forbidden`] if the user is not a member of `workspace_id`, or
/// [`ServerError::Transition`] (`403`) if their current role lacks permission for `action`.
pub async fn authorize_member<C: deadpool_postgres::GenericClient + Sync>(
    client: &C,
    workspace_id: &WorkspaceId,
    user_id: &UserId,
    action: WorkspaceAction,
) -> Result<WorkspaceRole, ServerError> {
    let row = client
        .query_opt(
            "SELECT role FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
            &[&workspace_id.as_ref(), &user_id.as_ref()],
        )
        .await?
        .ok_or_else(|| {
            ServerError::Forbidden(format!(
                "user {user_id} is not a member of workspace {workspace_id}"
            ))
        })?;

    let role_str: String = row.get("role");
    let role = WorkspaceRole::from_str_name(&role_str)
        .ok_or_else(|| ServerError::Internal(format!("invalid stored role: {role_str}")))?;
    authorize_workspace_action(role, action)?;
    Ok(role)
}
