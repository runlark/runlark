use crate::auth::authenticate_headers;
use crate::error::ServerError;
use crate::pg::sync;
use crate::server::ServerState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use runlark_domain::{
    CursorSeq, EventSeq, RunId, TeamRunEvent, WorkspaceEventEnvelope, WorkspaceId,
    WorkspaceSnapshot,
};
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct EventsCursorQuery {
    pub after_cursor: Option<u64>,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
pub struct RunEventsQuery {
    pub after_seq: Option<u64>,
}

/// Returns a consistent `REPEATABLE READ` [`WorkspaceSnapshot`] paired with its `cursor_seq`.
///
/// # Errors
/// Returns [`ServerError`] on auth denial or `PostgreSQL` error.
pub async fn get_snapshot(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(wid): Path<String>,
) -> Result<Json<WorkspaceSnapshot>, ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let snapshot =
        sync::get_workspace_snapshot(&state.pool, &user, &WorkspaceId::from(wid)).await?;
    Ok(Json(snapshot))
}

/// Returns incremental ordered [`WorkspaceEventEnvelope`] items strictly after `after_cursor`.
///
/// # Errors
/// Returns [`ServerError`] on auth denial or query failure.
pub async fn get_workspace_events(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(wid): Path<String>,
    Query(query): Query<EventsCursorQuery>,
) -> Result<Json<Vec<WorkspaceEventEnvelope>>, ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let events = sync::list_workspace_events(
        &state.pool,
        &user,
        &WorkspaceId::from(wid),
        CursorSeq(query.after_cursor.unwrap_or(0)),
        query.limit.unwrap_or(200),
    )
    .await?;
    Ok(Json(events))
}

/// Returns ordered per-run events strictly after `after_seq`.
///
/// # Errors
/// Returns [`ServerError`] on auth denial or missing run.
pub async fn get_run_events(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path((wid, run_id)): Path<(String, String)>,
    Query(query): Query<RunEventsQuery>,
) -> Result<Json<Vec<TeamRunEvent>>, ServerError> {
    let user = authenticate_headers(&state.pool, &headers).await?;
    let events = sync::list_run_events(
        &state.pool,
        &user,
        &WorkspaceId::from(wid),
        &RunId::from(run_id),
        query.after_seq.map(EventSeq),
    )
    .await?;
    Ok(Json(events))
}
