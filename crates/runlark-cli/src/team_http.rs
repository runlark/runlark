use crate::state::AppState;
use crate::team_sync::{TeamCommandSubmissionResult, TeamSyncWorker};
use reqwest::{Client, StatusCode};
use runlark_domain::{
    ArtifactDigest, ArtifactRef, CursorSeq, ProjectOwnershipMode, TeamCommand, Timestamp,
    WorkspaceEventEnvelope, WorkspaceId, WorkspaceSnapshot,
};
use runlark_local::{
    ArtifactCasStore, ProjectStore, StorageError, SyncStore, SyncedCasStore, open_sqlite,
    sha256_hex,
};
use serde::Deserialize;
use std::collections::HashSet;
use thiserror::Error;

/// Errors produced when communicating with a remote `runlark-server` or syncing local `SQLite`.
#[derive(Debug, Error)]
pub enum TeamServerError {
    #[error("Team server offline or unreachable: {0}")]
    Offline(String),

    #[error("Conflict ({code}): {message}")]
    Conflict {
        code: String,
        message: String,
        current_revision: Option<u64>,
    },

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize)]
struct ServerErrorEnvelope {
    error: Option<String>,
    message: Option<String>,
    current_revision: Option<u64>,
}

/// Parses a `reqwest::Response` from `runlark-server`, mapping structured HTTP errors into
/// [`TeamServerError`].
///
/// # Errors
/// Returns [`TeamServerError`] on non-2xx status or JSON decoding failure.
pub async fn parse_server_json_response<T: for<'de> Deserialize<'de>>(
    resp: reqwest::Response,
) -> Result<T, TeamServerError> {
    if !resp.status().is_success() {
        return Err(map_error_response(resp).await);
    }
    let text = resp
        .text()
        .await
        .map_err(|e| TeamServerError::Offline(e.to_string()))?;
    serde_json::from_str::<T>(&text).map_err(Into::into)
}

pub(crate) async fn map_error_response(resp: reqwest::Response) -> TeamServerError {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let parsed = serde_json::from_str::<ServerErrorEnvelope>(&text).ok();
    let code = parsed
        .as_ref()
        .and_then(|e| e.error.clone())
        .unwrap_or_else(|| "server_error".to_string());
    let message = parsed
        .as_ref()
        .and_then(|e| e.message.clone())
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| {
            if text.trim().is_empty() {
                format!("HTTP {status}")
            } else {
                text
            }
        });
    let current_revision = parsed.and_then(|e| e.current_revision);

    match status {
        StatusCode::CONFLICT => TeamServerError::Conflict {
            code,
            message,
            current_revision,
        },
        StatusCode::UNAUTHORIZED => TeamServerError::Unauthorized(message),
        StatusCode::FORBIDDEN => TeamServerError::Forbidden(message),
        StatusCode::NOT_FOUND => TeamServerError::NotFound(message),
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
            TeamServerError::BadRequest(message)
        }
        _ => TeamServerError::Offline(format!("server returned {status}: {message}")),
    }
}

/// Submits a [`TeamCommand`] to `POST /api/v1/workspaces/{wid}/commands`.
///
/// # Errors
/// Returns [`TeamServerError`] on HTTP/conflict/offline failure.
pub async fn post_team_command(
    http: &Client,
    server_url: &str,
    bearer_token: &str,
    workspace_id: &WorkspaceId,
    command: &TeamCommand,
) -> Result<TeamCommandSubmissionResult, TeamServerError> {
    let url = format!(
        "{}/api/v1/workspaces/{}/commands",
        server_url.trim_end_matches('/'),
        workspace_id.as_ref()
    );
    let resp = http
        .post(&url)
        .bearer_auth(bearer_token)
        .json(command)
        .send()
        .await
        .map_err(|e| TeamServerError::Offline(e.to_string()))?;
    parse_server_json_response(resp).await
}

/// Fetches a consistent `REPEATABLE READ` [`WorkspaceSnapshot`] from `runlark-server`.
///
/// # Errors
/// Returns [`TeamServerError`] on HTTP/offline/parse failure.
pub async fn fetch_snapshot(
    http: &Client,
    server_url: &str,
    bearer_token: &str,
    workspace_id: &WorkspaceId,
) -> Result<WorkspaceSnapshot, TeamServerError> {
    let url = format!(
        "{}/api/v1/workspaces/{}/sync/snapshot",
        server_url.trim_end_matches('/'),
        workspace_id.as_ref()
    );
    let resp = http
        .get(&url)
        .bearer_auth(bearer_token)
        .send()
        .await
        .map_err(|e| TeamServerError::Offline(e.to_string()))?;
    parse_server_json_response(resp).await
}

/// Fetches ordered [`WorkspaceEventEnvelope`] items strictly after `after_cursor`.
///
/// # Errors
/// Returns [`TeamServerError`] on HTTP/offline/parse failure.
pub async fn fetch_events(
    http: &Client,
    server_url: &str,
    bearer_token: &str,
    workspace_id: &WorkspaceId,
    after_cursor: CursorSeq,
    limit: u32,
) -> Result<Vec<WorkspaceEventEnvelope>, TeamServerError> {
    let url = format!(
        "{}/api/v1/workspaces/{}/sync/events?after_cursor={}&limit={limit}",
        server_url.trim_end_matches('/'),
        workspace_id.as_ref(),
        after_cursor.0
    );
    let resp = http
        .get(&url)
        .bearer_auth(bearer_token)
        .send()
        .await
        .map_err(|e| TeamServerError::Offline(e.to_string()))?;
    parse_server_json_response(resp).await
}

/// Uploads a verified CAS blob to `PUT /api/v1/workspaces/{wid}/artifacts/{digest}`.
///
/// # Errors
/// Returns [`TeamServerError`] on HTTP/offline/parse failure.
pub async fn upload_artifact(
    http: &Client,
    server_url: &str,
    bearer_token: &str,
    workspace_id: &WorkspaceId,
    meta: &ArtifactRef,
    bytes: &[u8],
) -> Result<ArtifactRef, TeamServerError> {
    let kind_str = serde_json::to_value(meta.kind)?
        .as_str()
        .unwrap_or("evidence_bundle")
        .to_string();
    let url = format!(
        "{}/api/v1/workspaces/{}/artifacts/{}",
        server_url.trim_end_matches('/'),
        workspace_id.as_ref(),
        meta.digest_sha256.as_str()
    );
    let resp = http
        .put(&url)
        .bearer_auth(bearer_token)
        .query(&[
            ("kind", kind_str.as_str()),
            ("media_type", meta.media_type.as_str()),
        ])
        .body(bytes.to_vec())
        .send()
        .await
        .map_err(|e| TeamServerError::Offline(e.to_string()))?;
    parse_server_json_response(resp).await
}

/// Downloads a CAS blob from `GET /api/v1/workspaces/{wid}/artifacts/{digest}` and verifies
/// its SHA-256 digest before returning bytes.
///
/// # Errors
/// Returns [`TeamServerError`] on HTTP/offline failure or SHA-256 mismatch.
pub async fn download_artifact(
    http: &Client,
    server_url: &str,
    bearer_token: &str,
    workspace_id: &WorkspaceId,
    digest: &ArtifactDigest,
) -> Result<Vec<u8>, TeamServerError> {
    let url = format!(
        "{}/api/v1/workspaces/{}/artifacts/{}",
        server_url.trim_end_matches('/'),
        workspace_id.as_ref(),
        digest.as_str()
    );
    let resp = http
        .get(&url)
        .bearer_auth(bearer_token)
        .send()
        .await
        .map_err(|e| TeamServerError::Offline(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(map_error_response(resp).await);
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| TeamServerError::Offline(e.to_string()))?
        .to_vec();
    let actual = sha256_hex(&bytes);
    if actual != digest.as_str() {
        return Err(TeamServerError::Storage(StorageError::CorruptArtifact {
            digest: digest.clone(),
            expected: digest.to_string(),
            actual,
        }));
    }
    Ok(bytes)
}

impl TeamSyncWorker {
    pub(crate) async fn upload_cas_for_workspace(
        &self,
        server_url: &str,
        bearer_token: &str,
        workspace_id: &WorkspaceId,
    ) -> Result<usize, TeamServerError> {
        let cas = self.state.cas_store();
        let CasTransferPlan { to_upload, .. } =
            plan_cas_transfers(&self.state, workspace_id, &cas)?;
        let mut uploaded = 0_usize;
        for (meta, bytes) in to_upload {
            upload_artifact(
                &self.http,
                server_url,
                bearer_token,
                workspace_id,
                &meta,
                &bytes,
            )
            .await?;
            let conn = open_sqlite(&self.state.db_path)?;
            SyncedCasStore::mark_uploaded(
                &conn,
                workspace_id,
                &meta.digest_sha256,
                Timestamp(AppState::current_timestamp_ms()),
            )?;
            uploaded += 1;
        }
        Ok(uploaded)
    }

    pub(crate) async fn download_cas_for_workspace(
        &self,
        server_url: &str,
        bearer_token: &str,
        workspace_id: &WorkspaceId,
    ) -> Result<usize, TeamServerError> {
        let cas = self.state.cas_store();
        let CasTransferPlan { to_download, .. } =
            plan_cas_transfers(&self.state, workspace_id, &cas)?;
        let mut downloaded = 0_usize;
        for meta in to_download {
            match download_artifact(
                &self.http,
                server_url,
                bearer_token,
                workspace_id,
                &meta.digest_sha256,
            )
            .await
            {
                Ok(bytes) => {
                    let conn = open_sqlite(&self.state.db_path)?;
                    cas.put_bytes(&conn, meta.kind, &meta.media_type, &bytes, meta.created_at)?;
                    SyncedCasStore::mark_uploaded(
                        &conn,
                        workspace_id,
                        &meta.digest_sha256,
                        Timestamp(AppState::current_timestamp_ms()),
                    )?;
                    downloaded += 1;
                }
                Err(TeamServerError::NotFound(_)) => {}
                Err(err) => return Err(err),
            }
        }
        Ok(downloaded)
    }
}

struct CasTransferPlan {
    to_upload: Vec<(ArtifactRef, Vec<u8>)>,
    to_download: Vec<ArtifactRef>,
}

fn plan_cas_transfers(
    state: &AppState,
    workspace_id: &WorkspaceId,
    cas: &ArtifactCasStore,
) -> Result<CasTransferPlan, TeamServerError> {
    let conn = open_sqlite(&state.db_path)?;
    let mut to_upload = Vec::new();
    let mut to_download = Vec::new();
    let mut seen = HashSet::new();

    for binding in SyncStore::list_project_bindings(&conn)? {
        let matches_ws = matches!(
            &binding.ownership_mode,
            ProjectOwnershipMode::TeamManaged {
                workspace_id: wid, ..
            } if wid == workspace_id
        );
        if !matches_ws {
            continue;
        }
        for run in ProjectStore::list_runs_for_project(&conn, &binding.project_id)? {
            for (meta, _) in ArtifactCasStore::list_artifacts_for_run(&conn, &run.id)? {
                if !seen.insert(meta.digest_sha256.clone()) {
                    continue;
                }
                if cas.has_blob(&meta.digest_sha256) {
                    if !SyncedCasStore::is_uploaded(&conn, workspace_id, &meta.digest_sha256)? {
                        let bytes = cas.get_bytes(&meta.digest_sha256)?;
                        to_upload.push((meta, bytes));
                    }
                } else {
                    to_download.push(meta);
                }
            }
        }
    }
    Ok(CasTransferPlan {
        to_upload,
        to_download,
    })
}
