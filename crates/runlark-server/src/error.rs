use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use runlark_domain::{DomainError, TransitionError};
use serde::Serialize;
use thiserror::Error;

/// Structured JSON error envelope returned by `runlark-server` endpoints.
#[derive(Debug, Serialize)]
pub struct ApiErrorBody {
    pub error: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_revision: Option<u64>,
}

/// Errors produced by `runlark-server` storage, authentication, CAS, or domain checks.
#[derive(Debug, Error)]
pub enum ServerError {
    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Conflict: {message}")]
    Conflict {
        code: &'static str,
        message: String,
        current_revision: Option<u64>,
    },

    #[error("Domain transition error: {0}")]
    Transition(#[from] TransitionError),

    #[error("Domain error: {0}")]
    Domain(#[from] DomainError),

    #[error("PostgreSQL error: {0}")]
    Postgres(#[from] tokio_postgres::Error),

    #[error("PostgreSQL pool error: {0}")]
    Pool(String),

    #[error("Serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl ServerError {
    /// Creates a `409 Conflict` error carrying an optional current aggregate revision.
    #[must_use]
    pub fn conflict(
        code: &'static str,
        message: impl Into<String>,
        current_revision: Option<u64>,
    ) -> Self {
        Self::Conflict {
            code,
            message: message.into(),
            current_revision,
        }
    }

    /// Converts a `u64` into `PostgreSQL` `BIGINT` (`i64`), rejecting overflows.
    ///
    /// # Errors
    /// Returns [`ServerError::BadRequest`] if the value exceeds `i64::MAX`.
    pub fn to_pg_i64(value: u64) -> Result<i64, Self> {
        i64::try_from(value)
            .map_err(|_| Self::BadRequest(format!("integer value {value} exceeds BIGINT range")))
    }

    /// Converts a `PostgreSQL` `BIGINT` (`i64`) into `u64`, rejecting negative values.
    ///
    /// # Errors
    /// Returns [`ServerError::BadRequest`] if the stored integer is negative.
    pub fn from_pg_i64(value: i64, field: &str) -> Result<u64, Self> {
        u64::try_from(value)
            .map_err(|_| Self::BadRequest(format!("negative BIGINT in field {field}: {value}")))
    }

    /// Converts a `u32` into `PostgreSQL` `INTEGER` (`i32`).
    ///
    /// # Errors
    /// Returns [`ServerError::BadRequest`] if the value exceeds `i32::MAX`.
    pub fn to_pg_i32(value: u32) -> Result<i32, Self> {
        i32::try_from(value)
            .map_err(|_| Self::BadRequest(format!("integer value {value} exceeds INTEGER range")))
    }

    /// Converts a `PostgreSQL` `INTEGER` (`i32`) into `u32`.
    ///
    /// # Errors
    /// Returns [`ServerError::BadRequest`] if the stored integer is negative.
    pub fn from_pg_i32(value: i32, field: &str) -> Result<u32, Self> {
        u32::try_from(value)
            .map_err(|_| Self::BadRequest(format!("negative INTEGER in field {field}: {value}")))
    }
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let (status, code, message, current_revision) = match self {
            Self::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, "unauthorized", msg, None),
            Self::Forbidden(msg) => (StatusCode::FORBIDDEN, "forbidden", msg, None),
            Self::NotFound(msg) => (StatusCode::NOT_FOUND, "not_found", msg, None),
            Self::BadRequest(msg) => (StatusCode::BAD_REQUEST, "bad_request", msg, None),
            Self::Conflict {
                code,
                message,
                current_revision,
            } => (StatusCode::CONFLICT, code, message, current_revision),
            Self::Transition(err) => match err {
                TransitionError::WorkspacePermissionDenied { .. } => (
                    StatusCode::FORBIDDEN,
                    "permission_denied",
                    err.to_string(),
                    None,
                ),
                TransitionError::RevisionMismatch { actual, .. } => (
                    StatusCode::CONFLICT,
                    "revision_mismatch",
                    err.to_string(),
                    Some(actual.0),
                ),
                TransitionError::RunnerFencingMismatch { .. } => (
                    StatusCode::CONFLICT,
                    "runner_fencing_mismatch",
                    err.to_string(),
                    None,
                ),
                TransitionError::ActiveRunPreventsTakeover { .. } => (
                    StatusCode::CONFLICT,
                    "active_run_prevents_takeover",
                    err.to_string(),
                    None,
                ),
                TransitionError::SoloToTeamBlockedByActiveRuns { .. } => (
                    StatusCode::CONFLICT,
                    "solo_to_team_blocked",
                    err.to_string(),
                    None,
                ),
                TransitionError::RunnerNotOnline { .. } => (
                    StatusCode::CONFLICT,
                    "runner_not_online",
                    err.to_string(),
                    None,
                ),
                other => (
                    StatusCode::CONFLICT,
                    "transition_error",
                    other.to_string(),
                    None,
                ),
            },
            Self::Domain(err) => (
                StatusCode::BAD_REQUEST,
                "domain_error",
                err.to_string(),
                None,
            ),
            Self::Postgres(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "postgres_error",
                err.to_string(),
                None,
            ),
            Self::Pool(msg) => (StatusCode::INTERNAL_SERVER_ERROR, "pool_error", msg, None),
            Self::Json(err) => (
                StatusCode::BAD_REQUEST,
                "invalid_json",
                err.to_string(),
                None,
            ),
            Self::Io(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "io_error",
                err.to_string(),
                None,
            ),
        };

        (
            status,
            Json(ApiErrorBody {
                error: code.to_string(),
                message,
                current_revision,
            }),
        )
            .into_response()
    }
}
