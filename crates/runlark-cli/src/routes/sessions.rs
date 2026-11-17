use crate::routes::error::{ApiError, api_error, run_status_str, storage_error_to_http};
use crate::routes::questions::QuestionResponse;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use runlark_domain::{Revision, Session, SessionId, SessionStatus, TaskId, Timestamp};
use runlark_local::{CommandStore, Connection, QuestionStore, SessionStore};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct CreateSessionRequest {
    pub session_id: String,
    pub task_id: String,
    pub cwd: String,
}

#[derive(Debug, Serialize)]
pub struct SessionResponse {
    pub session_id: String,
    pub task_id: String,
    pub cwd: String,
    pub status: String,
    pub current_revision: u64,
    pub active_run_id: Option<String>,
    pub active_run_status: Option<String>,
    pub worktree_path: Option<String>,
    pub latest_run_id: Option<String>,
    pub latest_run_status: Option<String>,
    pub latest_worktree_path: Option<String>,
    pub dispatched_commands_count: u64,
    pub pending_question: Option<QuestionResponse>,
    pub created_at: u64,
    pub updated_at: u64,
}

fn session_status_str(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Created => "created",
        SessionStatus::Active => "active",
        SessionStatus::Archived => "archived",
    }
}

fn enrich_session(conn: &Connection, session: Session) -> Result<SessionResponse, ApiError> {
    let active_run = match &session.active_run_id {
        Some(run_id) => SessionStore::get_run(conn, run_id).map_err(storage_error_to_http)?,
        None => None,
    };
    let latest_run = if let Some(ref active) = active_run {
        Some(active.clone())
    } else {
        SessionStore::list_runs_for_session(conn, &session.id)
            .map_err(storage_error_to_http)?
            .into_iter()
            .last()
    };
    let dispatched_commands_count = CommandStore::count_dispatched_for_session(conn, &session.id)
        .map_err(storage_error_to_http)?;
    let pending_questions = QuestionStore::list_pending_for_session(conn, &session.id)
        .map_err(storage_error_to_http)?;
    let pending_question = pending_questions
        .into_iter()
        .next()
        .map(QuestionResponse::from);

    Ok(SessionResponse {
        session_id: session.id.0,
        task_id: session.task_id.0,
        cwd: session.cwd,
        status: session_status_str(session.status).to_string(),
        current_revision: session.current_revision.0,
        active_run_id: session.active_run_id.map(|r| r.0),
        active_run_status: active_run
            .as_ref()
            .map(|r| run_status_str(r.status).to_string()),
        worktree_path: active_run.map(|r| r.worktree_path),
        latest_run_id: latest_run.as_ref().map(|r| r.id.0.clone()),
        latest_run_status: latest_run
            .as_ref()
            .map(|r| run_status_str(r.status).to_string()),
        latest_worktree_path: latest_run.map(|r| r.worktree_path),
        dispatched_commands_count,
        pending_question,
        created_at: session.created_at.0,
        updated_at: session.updated_at.0,
    })
}

/// Creates a new managed session.
///
/// # Errors
/// Returns `StatusCode::CONFLICT` if a session with the given ID already exists,
/// or `StatusCode::INTERNAL_SERVER_ERROR` if storage operation fails.
pub async fn create_session(
    State(state): State<AppState>,
    Json(payload): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<SessionResponse>), ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;

    let session_id = SessionId::from(payload.session_id);
    if SessionStore::get_session(&conn, &session_id)
        .map_err(storage_error_to_http)?
        .is_some()
    {
        return Err(api_error(
            StatusCode::CONFLICT,
            "session_already_exists",
            format!("Session {session_id} already exists"),
        ));
    }

    let now = AppState::current_timestamp_ms();
    let session = Session {
        id: session_id,
        task_id: TaskId::from(payload.task_id),
        cwd: payload.cwd,
        status: SessionStatus::Created,
        current_revision: Revision(1),
        active_run_id: None,
        created_at: Timestamp(now),
        updated_at: Timestamp(now),
    };

    SessionStore::create_session(&conn, &session).map_err(storage_error_to_http)?;
    state.notify_dsh_subscriber();
    let resp = SessionResponse {
        session_id: session.id.0,
        task_id: session.task_id.0,
        cwd: session.cwd,
        status: session_status_str(session.status).to_string(),
        current_revision: session.current_revision.0,
        active_run_id: None,
        active_run_status: None,
        worktree_path: None,
        latest_run_id: None,
        latest_run_status: None,
        latest_worktree_path: None,
        dispatched_commands_count: 0,
        pending_question: None,
        created_at: session.created_at.0,
        updated_at: session.updated_at.0,
    };
    Ok((StatusCode::CREATED, Json(resp)))
}

/// Lists all sessions enriched with active run status, worktree path, and pending questions.
///
/// # Errors
/// Returns `StatusCode::INTERNAL_SERVER_ERROR` if storage query fails.
pub async fn list_sessions(
    State(state): State<AppState>,
) -> Result<Json<Vec<SessionResponse>>, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let sessions = SessionStore::list_sessions(&conn).map_err(storage_error_to_http)?;

    let mut resp = Vec::with_capacity(sessions.len());
    for session in sessions {
        resp.push(enrich_session(&conn, session)?);
    }
    Ok(Json(resp))
}

/// Retrieves details for a specific session by ID.
///
/// # Errors
/// Returns `StatusCode::NOT_FOUND` if session does not exist,
/// or `StatusCode::INTERNAL_SERVER_ERROR` if storage query fails.
pub async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SessionResponse>, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let session_id = SessionId::from(id);
    let session = SessionStore::get_session(&conn, &session_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "session_not_found",
                format!("Session not found: {session_id}"),
            )
        })?;

    Ok(Json(enrich_session(&conn, session)?))
}
