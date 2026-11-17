use crate::dsh_events::{
    DshWireEventEnvelope, apply_runtime_event_and_reconcile, process_dsh_wire_event,
};
use crate::repair::WorkflowPipeline;
use crate::routes::error::{ApiError, api_error, run_status_str, storage_error_to_http};
use crate::routes::events::EventItemResponse;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use runlark_domain::{RuntimeEvent, SessionId, Timestamp, TurnEndStatus};
use runlark_local::SessionStore;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum IngestDshPayload {
    Runtime(RuntimeEvent),
    Wire(DshWireEventEnvelope),
}

#[derive(Debug, Serialize)]
pub struct DshEventIngestResponse {
    pub session_id: String,
    pub current_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_status: Option<String>,
    pub events: Vec<EventItemResponse>,
}

/// Ingests an external DSH runtime lifecycle event (`RuntimeEvent` or raw `SessionWireEvent`
/// envelope), updates durable state atomically, and reconciles dispatched outbox commands.
///
/// # Errors
/// Returns `StatusCode::NOT_FOUND` if session or run is missing,
/// `StatusCode::CONFLICT` / `StatusCode::UNPROCESSABLE_ENTITY` on invalid state transitions,
/// or `StatusCode::INTERNAL_SERVER_ERROR` if storage fails.
pub async fn ingest_dsh_event(
    State(state): State<AppState>,
    Json(payload): Json<IngestDshPayload>,
) -> Result<Json<DshEventIngestResponse>, ApiError> {
    let mut conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;

    let now = AppState::current_timestamp_ms();
    match payload {
        IngestDshPayload::Runtime(event) => {
            ingest_runtime_event(&mut conn, &state, &event, Timestamp(now)).await
        }
        IngestDshPayload::Wire(envelope) => {
            ingest_wire_event(&mut conn, &state, envelope, Timestamp(now)).await
        }
    }
}

async fn ingest_runtime_event(
    conn: &mut runlark_local::Connection,
    state: &AppState,
    event: &RuntimeEvent,
    now: Timestamp,
) -> Result<Json<DshEventIngestResponse>, ApiError> {
    let is_completed_turn = matches!(
        event,
        RuntimeEvent::TurnEnded {
            outcome: TurnEndStatus::Completed,
            ..
        }
    );
    let outcome = apply_runtime_event_and_reconcile(conn, state, event, now)
        .map_err(storage_error_to_http)?;
    if is_completed_turn && outcome.run.task_id.is_some() && !outcome.run.status.is_terminal() {
        WorkflowPipeline::on_dsh_turn_ended(
            state,
            &outcome.run.id,
            Timestamp(AppState::current_timestamp_ms()),
        )
        .await
        .map_err(|err| {
            tracing::error!(
                "workflow pipeline failed on runtime turn/end for {}: {err}",
                outcome.run.id
            );
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "workflow_pipeline_error",
                err.to_string(),
            )
        })?;
    }
    let latest_run = state
        .get_connection()
        .ok()
        .and_then(|c| SessionStore::get_run(&c, &outcome.run.id).ok().flatten())
        .unwrap_or(outcome.run);
    let events = outcome
        .events
        .into_iter()
        .map(EventItemResponse::from_stored)
        .collect();
    Ok(Json(DshEventIngestResponse {
        session_id: outcome.session.id.0,
        current_revision: outcome.session.current_revision.0,
        run_id: Some(latest_run.id.0),
        run_status: Some(run_status_str(latest_run.status).to_string()),
        events,
    }))
}

async fn ingest_wire_event(
    conn: &mut runlark_local::Connection,
    state: &AppState,
    envelope: DshWireEventEnvelope,
    now: Timestamp,
) -> Result<Json<DshEventIngestResponse>, ApiError> {
    let sid = SessionId::from(envelope.session_id);
    let is_turn_end = envelope.event.event_type == "turn/end";
    let stored = process_dsh_wire_event(
        conn,
        state,
        &sid,
        envelope.run_id.as_deref(),
        &envelope.event,
        now,
    )
    .map_err(storage_error_to_http)?;

    let session = SessionStore::get_session(conn, &sid)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "session_not_found",
                format!("Session not found: {sid}"),
            )
        })?;

    let mut run = match &session.active_run_id {
        Some(rid) => SessionStore::get_run(conn, rid).map_err(storage_error_to_http)?,
        None => SessionStore::list_runs_for_session(conn, &sid)
            .map_err(storage_error_to_http)?
            .into_iter()
            .last(),
    };
    if is_turn_end
        && let Some(ref active_run) = run
        && active_run.task_id.is_some()
        && !active_run.status.is_terminal()
    {
        WorkflowPipeline::on_dsh_turn_ended(
            state,
            &active_run.id,
            Timestamp(AppState::current_timestamp_ms()),
        )
        .await
        .map_err(|err| {
            tracing::error!(
                "workflow pipeline failed on wire turn/end for {}: {err}",
                active_run.id
            );
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "workflow_pipeline_error",
                err.to_string(),
            )
        })?;
        if let Ok(c) = state.get_connection()
            && let Ok(Some(refreshed)) = SessionStore::get_run(&c, &active_run.id)
        {
            run = Some(refreshed);
        }
    }

    Ok(Json(DshEventIngestResponse {
        session_id: session.id.0,
        current_revision: session.current_revision.0,
        run_id: run.as_ref().map(|r| r.id.0.clone()),
        run_status: run.as_ref().map(|r| run_status_str(r.status).to_string()),
        events: stored
            .into_iter()
            .map(EventItemResponse::from_stored)
            .collect(),
    }))
}
