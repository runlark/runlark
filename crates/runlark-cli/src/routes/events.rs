use crate::routes::error::{ApiError, api_error, storage_error_to_http};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use runlark_domain::{EventSeq, RunId, SessionId};
use runlark_local::{EventStore, SessionStore, StoredEvent};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

const MAX_WAIT_MS: u64 = 30_000;

#[derive(Deserialize)]
pub struct EventsQuery {
    pub after_seq: Option<u64>,
    pub wait_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct EventItemResponse {
    pub run_id: String,
    pub seq: u64,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: u64,
}

impl EventItemResponse {
    #[must_use]
    pub fn from_stored(event: StoredEvent) -> Self {
        let json_payload = serde_json::from_str(&event.payload)
            .unwrap_or(serde_json::Value::String(event.payload));
        Self {
            run_id: event.run_id.0,
            seq: event.seq.0,
            event_type: event.event_type,
            payload: json_payload,
            created_at: event.created_at.0,
        }
    }
}

/// Retrieves events for a specific run execution, optionally after `after_seq` and with optional
/// long-polling (`wait_ms`, capped at 30s) until a matching event is persisted.
///
/// # Errors
/// Returns `StatusCode::INTERNAL_SERVER_ERROR` if querying events from storage fails.
pub async fn get_events(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<Vec<EventItemResponse>>, ApiError> {
    let after_seq = query.after_seq.map(EventSeq);
    let wait_ms = query.wait_ms.unwrap_or(0).min(MAX_WAIT_MS);
    let target_run = RunId::from(run_id.clone());

    let mut rx = (wait_ms > 0).then(|| state.subscribe_events());

    let initial = {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        EventStore::get_events(&conn, &target_run, after_seq).map_err(storage_error_to_http)?
    };

    if !initial.is_empty() || wait_ms == 0 {
        return Ok(Json(
            initial
                .into_iter()
                .map(EventItemResponse::from_stored)
                .collect(),
        ));
    }

    if let Some(ref mut receiver) = rx {
        let mut shutdown = state.subscribe_shutdown();
        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        loop {
            if *shutdown.borrow() {
                break;
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            if remaining.is_zero() {
                break;
            }
            tokio::select! {
                res = tokio::time::timeout(remaining, receiver.recv()) => {
                    match res {
                        Ok(Ok(note)) if note.run_id == run_id => break,
                        Ok(Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
                        Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) | Err(_) => break,
                    }
                }
                _ = shutdown.changed() => break,
            }
        }
    }

    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let events =
        EventStore::get_events(&conn, &target_run, after_seq).map_err(storage_error_to_http)?;
    Ok(Json(
        events
            .into_iter()
            .map(EventItemResponse::from_stored)
            .collect(),
    ))
}

fn resolve_session_run_id(
    conn: &runlark_local::Connection,
    sid: &SessionId,
) -> Result<Option<RunId>, ApiError> {
    let session = SessionStore::get_session(conn, sid)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "session_not_found",
                format!("Session not found: {sid}"),
            )
        })?;

    if let Some(id) = session.active_run_id {
        return Ok(Some(id));
    }

    Ok(SessionStore::list_runs_for_session(conn, sid)
        .map_err(storage_error_to_http)?
        .into_iter()
        .last()
        .map(|r| r.id))
}

/// Retrieves events for a session's active (or most recent) run, supporting `after_seq` and `wait_ms`.
///
/// # Errors
/// Returns `StatusCode::NOT_FOUND` if the session does not exist,
/// or `StatusCode::INTERNAL_SERVER_ERROR` if querying storage fails.
pub async fn get_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<Vec<EventItemResponse>>, ApiError> {
    let after_seq = query.after_seq.map(EventSeq);
    let wait_ms = query.wait_ms.unwrap_or(0).min(MAX_WAIT_MS);
    let sid = SessionId::from(session_id.clone());

    let mut rx = (wait_ms > 0).then(|| state.subscribe_events());

    let initial_run_id = {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        resolve_session_run_id(&conn, &sid)?
    };

    if let Some(run_id) = initial_run_id {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        let initial =
            EventStore::get_events(&conn, &run_id, after_seq).map_err(storage_error_to_http)?;
        if !initial.is_empty() || wait_ms == 0 {
            return Ok(Json(
                initial
                    .into_iter()
                    .map(EventItemResponse::from_stored)
                    .collect(),
            ));
        }
    } else if wait_ms == 0 {
        return Ok(Json(Vec::new()));
    }

    if let Some(ref mut receiver) = rx {
        let mut shutdown = state.subscribe_shutdown();
        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        loop {
            if *shutdown.borrow() {
                break;
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            if remaining.is_zero() {
                break;
            }
            tokio::select! {
                res = tokio::time::timeout(remaining, receiver.recv()) => {
                    match res {
                        Ok(Ok(note)) if note.session_id == session_id => break,
                        Ok(Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
                        Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) | Err(_) => break,
                    }
                }
                _ = shutdown.changed() => break,
            }
        }
    }

    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let Some(run_id) = resolve_session_run_id(&conn, &sid)? else {
        return Ok(Json(Vec::new()));
    };
    let events =
        EventStore::get_events(&conn, &run_id, after_seq).map_err(storage_error_to_http)?;
    Ok(Json(
        events
            .into_iter()
            .map(EventItemResponse::from_stored)
            .collect(),
    ))
}
