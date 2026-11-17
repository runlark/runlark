use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use runlark_domain::{SessionCommand, Timestamp};
use runlark_local::{CommandStore, PersistOutcome, StorageError};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize)]
pub struct SubmitCommandResponse {
    pub command_id: String,
    pub status: String,
}

fn current_timestamp() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

/// Submits and persists a command intent before external dispatch.
///
/// # Errors
/// Returns `StatusCode::CONFLICT` if the command exists with different payload,
/// or `StatusCode::INTERNAL_SERVER_ERROR` if storage operations fail.
pub async fn submit_command(
    State(state): State<AppState>,
    Json(command): Json<SessionCommand>,
) -> Result<(StatusCode, Json<SubmitCommandResponse>), (StatusCode, String)> {
    let mut conn = state
        .get_connection()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let now = current_timestamp();
    let command_id = command.command_id().to_string();

    match CommandStore::persist_intent(&mut conn, &command, Timestamp(now)) {
        Ok(PersistOutcome::Inserted) => Ok((
            StatusCode::CREATED,
            Json(SubmitCommandResponse {
                command_id,
                status: "pending".to_string(),
            }),
        )),
        Ok(PersistOutcome::AlreadyExists { status }) => Ok((
            StatusCode::OK,
            Json(SubmitCommandResponse { command_id, status }),
        )),
        Err(StorageError::CommandPayloadConflict(_)) => Err((
            StatusCode::CONFLICT,
            "Command payload conflict: same command_id with different payload".to_string(),
        )),
        Err(err) => Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
    }
}
