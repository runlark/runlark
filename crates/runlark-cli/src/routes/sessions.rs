use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use runlark_domain::{Revision, Session, SessionId, SessionStatus, TaskId, Timestamp};
use runlark_local::SessionStore;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Deserialize)]
pub struct CreateSessionRequest {
    pub session_id: String,
    pub task_id: String,
    pub cwd: String,
}

#[derive(Serialize)]
pub struct SessionResponse {
    pub session_id: String,
    pub task_id: String,
    pub cwd: String,
    pub status: String,
    pub current_revision: u64,
    pub active_run_id: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl From<Session> for SessionResponse {
    fn from(s: Session) -> Self {
        let status = match s.status {
            SessionStatus::Created => "created",
            SessionStatus::Active => "active",
            SessionStatus::Archived => "archived",
        };
        Self {
            session_id: s.id.0,
            task_id: s.task_id.0,
            cwd: s.cwd,
            status: status.to_string(),
            current_revision: s.current_revision.0,
            active_run_id: s.active_run_id.map(|r| r.0),
            created_at: s.created_at.0,
            updated_at: s.updated_at.0,
        }
    }
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

/// Creates a new managed session.
///
/// # Errors
/// Returns `StatusCode::INTERNAL_SERVER_ERROR` if storage operation fails.
pub async fn create_session(
    State(state): State<AppState>,
    Json(payload): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<SessionResponse>), (StatusCode, String)> {
    let conn = state
        .get_connection()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let now = current_timestamp();

    let session = Session {
        id: SessionId::from(payload.session_id),
        task_id: TaskId::from(payload.task_id),
        cwd: payload.cwd,
        status: SessionStatus::Created,
        current_revision: Revision(1),
        active_run_id: None,
        created_at: Timestamp(now),
        updated_at: Timestamp(now),
    };

    SessionStore::create_session(&conn, &session)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(SessionResponse::from(session))))
}

/// Lists all sessions.
///
/// # Errors
/// Returns `StatusCode::INTERNAL_SERVER_ERROR` if storage query fails.
pub async fn list_sessions(
    State(state): State<AppState>,
) -> Result<Json<Vec<SessionResponse>>, (StatusCode, String)> {
    let conn = state
        .get_connection()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let sessions = SessionStore::list_sessions(&conn)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let resp = sessions.into_iter().map(SessionResponse::from).collect();
    Ok(Json(resp))
}

/// Retrieves details for a specific session by ID.
///
/// # Errors
/// Returns `StatusCode::NOT_FOUND` if session does not exist, or `StatusCode::INTERNAL_SERVER_ERROR` if storage query fails.
pub async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SessionResponse>, (StatusCode, String)> {
    let conn = state
        .get_connection()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let session = SessionStore::get_session(&conn, &SessionId::from(id))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Session not found".to_string()))?;

    Ok(Json(SessionResponse::from(session)))
}
