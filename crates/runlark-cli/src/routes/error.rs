use axum::Json;
use axum::http::StatusCode;
use runlark_domain::{RunStatus, TransitionError};
use runlark_local::StorageError;
use serde::Serialize;

/// Structured JSON error payload returned by daemon HTTP endpoints.
#[derive(Debug, Serialize)]
pub struct ApiErrorBody {
    pub error: &'static str,
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
            message: message.into(),
            expected_revision: None,
            actual_revision: None,
        }),
    )
}

/// Maps a [`StorageError`] (including domain [`TransitionError`]) into a structured HTTP status and JSON body.
pub fn storage_error_to_http(err: StorageError) -> ApiError {
    match err {
        StorageError::CommandPayloadConflict(id) => api_error(
            StatusCode::CONFLICT,
            "command_payload_conflict",
            format!("Command payload conflict: command {id} exists with differing payload"),
        ),
        StorageError::SessionNotFound(id) => api_error(
            StatusCode::NOT_FOUND,
            "session_not_found",
            format!("Session not found: {id}"),
        ),
        StorageError::RunNotFound(id) => api_error(
            StatusCode::NOT_FOUND,
            "run_not_found",
            format!("Run not found: {id}"),
        ),
        StorageError::QuestionNotFound(id) => api_error(
            StatusCode::NOT_FOUND,
            "question_not_found",
            format!("Question not found: {id}"),
        ),
        StorageError::Transition(TransitionError::RevisionMismatch { expected, actual }) => (
            StatusCode::CONFLICT,
            Json(ApiErrorBody {
                error: "revision_mismatch",
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
