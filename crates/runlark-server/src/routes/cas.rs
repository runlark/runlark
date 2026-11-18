use crate::auth::authenticate_headers;
use crate::config::ServerConfig;
use crate::error::ServerError;
use crate::pg::cas;
use crate::server::ServerState;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use runlark_domain::{ArtifactDigest, ArtifactKind, ArtifactRef, Timestamp, WorkspaceId};
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct UploadArtifactQuery {
    pub kind: Option<ArtifactKind>,
    pub media_type: Option<String>,
}

/// Uploads an immutable CAS artifact blob, verifying its SHA-256 hash matches `{digest}`.
///
/// # Errors
/// Returns [`ServerError::BadRequest`] on invalid or mismatched SHA-256, or `403` on RBAC denial.
pub async fn upload_artifact(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path((wid, digest_raw)): Path<(String, String)>,
    Query(query): Query<UploadArtifactQuery>,
    body: Bytes,
) -> Result<(StatusCode, Json<ArtifactRef>), ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let digest = ArtifactDigest::parse(digest_raw)?;
    let kind = query.kind.unwrap_or(ArtifactKind::EvidenceBundle);
    let media_type = query
        .media_type
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("application/octet-stream");
    let now = Timestamp(ServerConfig::current_timestamp_ms());

    let ws_id = WorkspaceId::from(wid);
    cas::authorize_workspace_action(
        &state.pool,
        &user,
        &ws_id,
        runlark_domain::WorkspaceAction::MutateTasks,
    )
    .await?;
    let meta = state
        .cas
        .put_verified_bytes(&digest, kind, media_type, &body, now)?;
    let recorded = cas::record_artifact_metadata(&state.pool, &user, &ws_id, &meta).await?;
    Ok((StatusCode::CREATED, Json(recorded)))
}

/// Downloads a CAS artifact blob after checking membership and verifying its SHA-256 hash.
///
/// # Errors
/// Returns [`ServerError::NotFound`] if missing, [`ServerError::BadRequest`] on digest corruption.
pub async fn download_artifact(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path((wid, digest_raw)): Path<(String, String)>,
) -> Result<impl IntoResponse, ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let digest = ArtifactDigest::parse(digest_raw)?;
    let meta =
        cas::get_artifact_metadata(&state.pool, &user, &WorkspaceId::from(wid), &digest, None)
            .await?;
    let bytes = state.cas.get_verified_bytes(&digest)?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, meta.media_type)],
        bytes,
    ))
}
