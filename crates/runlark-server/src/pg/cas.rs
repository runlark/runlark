use crate::auth::{AuthenticatedUser, authorize_member};
use crate::error::ServerError;
use crate::pg::codec::{artifact_kind_to_str, row_to_artifact_ref};
use crate::pg::{get_client, lock_workspace_write};
use deadpool_postgres::Pool;
use runlark_domain::{ArtifactDigest, ArtifactKind, ArtifactRef, WorkspaceAction, WorkspaceId};

/// Verifies the caller's current workspace role permits `action` before side effects run.
///
/// # Errors
/// Returns [`ServerError::Forbidden`] if not a member or role lacks permission.
pub async fn authorize_workspace_action(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
    action: WorkspaceAction,
) -> Result<(), ServerError> {
    let client = get_client(pool).await?;
    authorize_member(&client, workspace_id, &actor.user_id, action).await?;
    Ok(())
}

/// Persists workspace-scoped CAS artifact metadata in `PostgreSQL` after verified disk blob
/// persistence, rejecting conflicting `(kind, media_type)` re-uploads within the same workspace.
///
/// # Errors
/// Returns [`ServerError`] on RBAC denial, workspace metadata conflict (`409`), or SQL error.
pub async fn record_artifact_metadata(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
    artifact: &ArtifactRef,
) -> Result<ArtifactRef, ServerError> {
    let mut client = get_client(pool).await?;
    let created_i64 = ServerError::to_pg_i64(artifact.created_at.0)?;
    let tx = client.transaction().await?;
    lock_workspace_write(&tx, workspace_id).await?;
    authorize_member(
        &tx,
        workspace_id,
        &actor.user_id,
        WorkspaceAction::MutateTasks,
    )
    .await?;

    tx.execute(
        concat!(
            "INSERT INTO team_artifacts (digest_sha256, size_bytes, created_at) ",
            "VALUES ($1, $2, $3) ",
            "ON CONFLICT (digest_sha256) DO NOTHING",
        ),
        &[
            &artifact.digest_sha256.as_str(),
            &ServerError::to_pg_i64(artifact.size_bytes)?,
            &created_i64,
        ],
    )
    .await?;

    let existing_row = tx
        .query_opt(
            concat!(
                "SELECT a.digest_sha256, wa.kind, wa.media_type, a.size_bytes, wa.created_at ",
                "FROM team_workspace_artifacts wa ",
                "INNER JOIN team_artifacts a ON a.digest_sha256 = wa.digest_sha256 ",
                "WHERE wa.workspace_id = $1 AND wa.digest_sha256 = $2 ",
                "FOR UPDATE OF wa",
            ),
            &[&workspace_id.as_ref(), &artifact.digest_sha256.as_str()],
        )
        .await?;

    let recorded = if let Some(row) = existing_row {
        let existing = row_to_artifact_ref(&row)?;
        if existing.kind != artifact.kind || existing.media_type != artifact.media_type {
            return Err(ServerError::conflict(
                "artifact_metadata_conflict",
                format!(
                    "artifact {} already exists in workspace with kind {:?} and media_type {:?}",
                    artifact.digest_sha256, existing.kind, existing.media_type
                ),
                None,
            ));
        }
        existing
    } else {
        tx.execute(
            concat!(
                "INSERT INTO team_workspace_artifacts ",
                "(workspace_id, digest_sha256, kind, media_type, created_at) ",
                "VALUES ($1, $2, $3, $4, $5)",
            ),
            &[
                &workspace_id.as_ref(),
                &artifact.digest_sha256.as_str(),
                &artifact_kind_to_str(artifact.kind),
                &artifact.media_type,
                &created_i64,
            ],
        )
        .await?;
        artifact.clone()
    };

    tx.commit().await?;
    Ok(recorded)
}

/// Checks read membership and fetches workspace-scoped CAS artifact metadata by SHA-256 digest.
///
/// # Errors
/// Returns [`ServerError::NotFound`] if metadata is missing in this workspace or [`ServerError::Forbidden`].
pub async fn get_artifact_metadata(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
    digest: &ArtifactDigest,
    expected_kind: Option<ArtifactKind>,
) -> Result<ArtifactRef, ServerError> {
    let client = get_client(pool).await?;
    authorize_member(
        &client,
        workspace_id,
        &actor.user_id,
        WorkspaceAction::ReadWorkspace,
    )
    .await?;

    let row = client
        .query_opt(
            concat!(
                "SELECT a.digest_sha256, wa.kind, wa.media_type, a.size_bytes, wa.created_at ",
                "FROM team_artifacts a ",
                "INNER JOIN team_workspace_artifacts wa ON wa.digest_sha256 = a.digest_sha256 ",
                "WHERE wa.workspace_id = $1 AND a.digest_sha256 = $2",
            ),
            &[&workspace_id.as_ref(), &digest.as_str()],
        )
        .await?
        .ok_or_else(|| {
            ServerError::NotFound(format!("artifact metadata not found: {}", digest.as_str()))
        })?;
    let meta = row_to_artifact_ref(&row)?;
    if let Some(kind) = expected_kind
        && meta.kind != kind
    {
        return Err(ServerError::BadRequest(format!(
            "artifact kind mismatch: expected {kind:?}, got {:?}",
            meta.kind
        )));
    }
    Ok(meta)
}
