use crate::chat::{ChatDispatchRequest, ChatDispatchResponse, dispatch_chat_command};
use crate::routes::error::ApiError;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

/// Parses and executes a slash command or prompt via the unified chat command layer.
///
/// # Errors
/// Returns `400` on invalid slash command syntax, `404` if target missing, or `409` on conflict.
pub async fn chat_dispatch(
    State(state): State<AppState>,
    Json(req): Json<ChatDispatchRequest>,
) -> Result<(StatusCode, Json<ChatDispatchResponse>), ApiError> {
    let (status, resp) = dispatch_chat_command(state, req).await?;
    Ok((status, Json(resp)))
}
