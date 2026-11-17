use crate::routes::error::{ApiError, api_error, run_status_str, storage_error_to_http};
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use runlark_domain::{SessionCommand, Timestamp};
use runlark_local::{CommandApplyOutcome, LocalStore};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct SubmitCommandResponse {
    pub command_id: String,
    pub status: String,
    pub current_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_status: Option<String>,
}

/// Submits, persists, and atomically applies a [`SessionCommand`] before notifying the outbox.
///
/// # Errors
/// Returns:
/// - `409 Conflict` on `RevisionMismatch`, `CommandPayloadConflict`, or state conflict transitions
/// - `404 Not Found` if the target session, run, or question does not exist
/// - `422 Unprocessable Entity` on invalid state/session mismatch
/// - `500 Internal Server Error` if opening or writing `SQLite` fails
pub async fn submit_command(
    State(state): State<AppState>,
    Json(command): Json<SessionCommand>,
) -> Result<(StatusCode, Json<SubmitCommandResponse>), ApiError> {
    let mut conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;

    let now = AppState::current_timestamp_ms();
    let command_id = command.command_id().to_string();

    let outcome = LocalStore::apply_command(&mut conn, &command, Timestamp(now))
        .map_err(storage_error_to_http)?;

    match outcome {
        CommandApplyOutcome::Applied {
            status,
            session,
            run,
            events,
        } => {
            state.publish_events(&session.id.0, &events);
            state.notify_outbox();
            state.notify_dsh_subscriber();
            state.notify_scheduler();
            Ok((
                StatusCode::CREATED,
                Json(SubmitCommandResponse {
                    command_id,
                    status,
                    current_revision: session.current_revision.0,
                    run_id: run.as_ref().map(|r| r.id.0.clone()),
                    run_status: run.as_ref().map(|r| run_status_str(r.status).to_string()),
                }),
            ))
        }
        CommandApplyOutcome::AlreadyExists {
            status,
            session,
            run,
        } => Ok((
            StatusCode::OK,
            Json(SubmitCommandResponse {
                command_id,
                status,
                current_revision: session.current_revision.0,
                run_id: run.as_ref().map(|r| r.id.0.clone()),
                run_status: run.as_ref().map(|r| run_status_str(r.status).to_string()),
            }),
        )),
    }
}
