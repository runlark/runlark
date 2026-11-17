use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use runlark_domain::{EventSeq, RunId};
use runlark_local::EventStore;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct EventsQuery {
    pub after_seq: Option<u64>,
}

#[derive(Serialize)]
pub struct EventItemResponse {
    pub seq: u64,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: u64,
}

/// Retrieves events for a specific run execution, optionally after a given sequence number.
///
/// # Errors
/// Returns `StatusCode::INTERNAL_SERVER_ERROR` if querying events from storage fails.
pub async fn get_events(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<Vec<EventItemResponse>>, (StatusCode, String)> {
    let conn = state
        .get_connection()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let after_seq = query.after_seq.map(EventSeq);
    let events = EventStore::get_events(&conn, &RunId::from(run_id), after_seq)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let resp = events
        .into_iter()
        .map(|event| {
            let json_payload = serde_json::from_str(&event.payload)
                .unwrap_or(serde_json::Value::String(event.payload));
            EventItemResponse {
                seq: event.seq.0,
                event_type: event.event_type,
                payload: json_payload,
                created_at: event.created_at.0,
            }
        })
        .collect();

    Ok(Json(resp))
}
