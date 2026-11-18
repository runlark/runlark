use crate::auth::authenticate_headers;
use crate::config::ServerConfig;
use crate::error::ServerError;
use crate::pg::commands::{self, TeamCommandResult};
use crate::pg::sync;
use crate::server::ServerState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use runlark_domain::{TaskComment, TaskId, TeamCommand, Timestamp, WorkspaceId};

/// Submits an ordered, idempotent [`TeamCommand`] to `wid`.
///
/// Returns `201 Created` on fresh execution and `200 OK` on idempotent replay.
///
/// # Errors
/// Returns [`ServerError`] on RBAC denial (`403`), payload/revision conflict (`409`), or DB failure.
pub async fn submit_command(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(wid): Path<String>,
    Json(command): Json<TeamCommand>,
) -> Result<(StatusCode, Json<TeamCommandResult>), ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let now = Timestamp(ServerConfig::current_timestamp_ms());
    let result = commands::apply_team_command(
        &state.pool,
        &state.cas,
        &user,
        &WorkspaceId::from(wid),
        command,
        now,
    )
    .await?;
    let code = if result.idempotent_replay {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((code, Json(result)))
}

/// Lists immutable append-only comments on `task_id`.
///
/// # Errors
/// Returns [`ServerError`] on auth/membership denial.
pub async fn list_comments(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path((wid, task_id)): Path<(String, String)>,
) -> Result<Json<Vec<TaskComment>>, ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let comments = sync::list_task_comments(
        &state.pool,
        &user,
        &WorkspaceId::from(wid),
        &TaskId::from(task_id),
    )
    .await?;
    Ok(Json(comments))
}
