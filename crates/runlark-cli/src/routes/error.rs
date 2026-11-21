use crate::team_sync::TeamServerError;
use axum::Json;
use axum::http::StatusCode;
use runlark_domain::{RunStatus, TransitionError};
use runlark_local::{StorageError, sha256_hex};
use serde::Serialize;

/// Structured JSON error payload returned by daemon HTTP endpoints.
#[derive(Debug, Serialize)]
pub struct ApiErrorBody {
    pub error: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_code: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_revision: Option<u64>,
}

/// Standard HTTP error tuple returned by route handlers.
pub type ApiError = (StatusCode, Json<ApiErrorBody>);

/// Builds an [`ApiError`] with no revision metadata.
pub fn api_error(status: StatusCode, error: &'static str, message: impl Into<String>) -> ApiError {
    (
        status,
        Json(ApiErrorBody {
            error,
            server_code: None,
            message: message.into(),
            expected_revision: None,
            actual_revision: None,
        }),
    )
}

/// Maps a [`StorageError`] (including domain [`TransitionError`]) into a structured HTTP status and JSON body.
pub fn storage_error_to_http(err: StorageError) -> ApiError {
    if let Some((code, msg)) = storage_not_found_pair(&err) {
        return api_error(StatusCode::NOT_FOUND, code, msg);
    }
    match err {
        StorageError::CommandPayloadConflict(id) => api_error(
            StatusCode::CONFLICT,
            "command_payload_conflict",
            format!("Command payload conflict: command {id} exists with differing payload"),
        ),
        StorageError::CursorGap { expected, actual } => api_error(
            StatusCode::CONFLICT,
            "cursor_gap",
            format!("Workspace cursor gap: expected {expected}, got {actual}"),
        ),
        StorageError::CorruptArtifact { digest, .. } => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "corrupt_artifact",
            format!("Corrupt CAS artifact: {digest}"),
        ),
        StorageError::Domain(e) => api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "domain_error",
            e.to_string(),
        ),
        StorageError::Io(e) => {
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "io_error", e.to_string())
        }
        StorageError::Transition(TransitionError::RevisionMismatch { expected, actual }) => (
            StatusCode::CONFLICT,
            Json(ApiErrorBody {
                error: "revision_mismatch",
                server_code: Some("revision_mismatch".to_string()),
                message: format!("Revision mismatch: expected {expected}, actual {actual}"),
                expected_revision: Some(expected.0),
                actual_revision: Some(actual.0),
            }),
        ),
        StorageError::Transition(
            te @ (TransitionError::SessionMismatch { .. } | TransitionError::NoActiveRun),
        ) => api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_transition",
            te.to_string(),
        ),
        StorageError::Transition(te) => {
            api_error(StatusCode::CONFLICT, "transition_conflict", te.to_string())
        }
        StorageError::InvalidState(msg) => {
            api_error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_state", msg)
        }
        StorageError::Sqlite(e) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        ),
        StorageError::Serialization(e) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialization_error",
            e.to_string(),
        ),
        _ => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            err.to_string(),
        ),
    }
}

fn storage_not_found_pair(err: &StorageError) -> Option<(&'static str, String)> {
    match err {
        StorageError::SessionNotFound(id) => {
            Some(("session_not_found", format!("Session not found: {id}")))
        }
        StorageError::RunNotFound(id) => Some(("run_not_found", format!("Run not found: {id}"))),
        StorageError::QuestionNotFound(id) => {
            Some(("question_not_found", format!("Question not found: {id}")))
        }
        StorageError::ProjectNotFound(id) => {
            Some(("project_not_found", format!("Project not found: {id}")))
        }
        StorageError::TaskNotFound(id) => Some(("task_not_found", format!("Task not found: {id}"))),
        StorageError::DecisionNotFound(id) => {
            Some(("decision_not_found", format!("Decision not found: {id}")))
        }
        StorageError::DraftNotFound(id) => {
            Some(("draft_not_found", format!("Offline draft not found: {id}")))
        }
        StorageError::SyncOutboxNotFound(id) => Some((
            "sync_outbox_not_found",
            format!("Sync outbox command not found: {id}"),
        )),
        StorageError::ArtifactNotFound(digest) => Some((
            "artifact_not_found",
            format!("Artifact not found: {digest}"),
        )),
        _ => None,
    }
}

/// Maps a [`TeamServerError`] into a structured daemon HTTP status and JSON body.
pub fn team_server_error_to_http(err: TeamServerError) -> ApiError {
    match err {
        TeamServerError::Offline(msg) => {
            api_error(StatusCode::SERVICE_UNAVAILABLE, "team_server_offline", msg)
        }
        TeamServerError::Conflict {
            code,
            message,
            current_revision,
        } => (
            StatusCode::CONFLICT,
            Json(ApiErrorBody {
                error: "team_conflict",
                server_code: Some(code),
                message,
                expected_revision: None,
                actual_revision: current_revision,
            }),
        ),
        TeamServerError::Unauthorized(msg) => {
            api_error(StatusCode::UNAUTHORIZED, "team_unauthorized", msg)
        }
        TeamServerError::Forbidden(msg) => api_error(StatusCode::FORBIDDEN, "team_forbidden", msg),
        TeamServerError::NotFound(msg) => api_error(StatusCode::NOT_FOUND, "team_not_found", msg),
        TeamServerError::BadRequest(msg) => {
            api_error(StatusCode::BAD_REQUEST, "team_bad_request", msg)
        }
        TeamServerError::Storage(e) => storage_error_to_http(e),
        TeamServerError::Json(e) => {
            api_error(StatusCode::BAD_GATEWAY, "team_json_error", e.to_string())
        }
    }
}

/// Converts a [`RunStatus`] into its canonical snake-case string representation.
#[must_use]
pub const fn run_status_str(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Queued => "queued",
        RunStatus::Running => "running",
        RunStatus::WaitingForApproval => "waiting_for_approval",
        RunStatus::CancelRequested => "cancel_requested",
        RunStatus::Stopped => "stopped",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
        RunStatus::Interrupted => "interrupted",
    }
}

/// Derives a deterministic short entity identifier from `command_id` for idempotent retries
/// when a client supplies `command_id` without specifying an explicit entity ID.
#[must_use]
pub fn deterministic_id(prefix: &str, command_id: &str) -> String {
    let digest = sha256_hex(command_id.as_bytes());
    let short = &digest[..16];
    format!("{prefix}_{short}")
}
