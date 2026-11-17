use crate::routes::error::{ApiError, api_error, run_status_str, storage_error_to_http};
use crate::routes::questions::QuestionResponse;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use runlark_domain::{Run, RunId};
use runlark_local::{QuestionStore, SessionStore};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct RunResponse {
    pub run_id: String,
    pub session_id: String,
    pub status: String,
    pub worktree_path: String,
    pub revision: u64,
    pub pending_questions: Vec<QuestionResponse>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl RunResponse {
    #[must_use]
    pub fn from_run(run: Run, pending_questions: Vec<QuestionResponse>) -> Self {
        Self {
            run_id: run.id.0,
            session_id: run.session_id.0,
            status: run_status_str(run.status).to_string(),
            worktree_path: run.worktree_path,
            revision: run.revision.0,
            pending_questions,
            created_at: run.created_at.0,
            updated_at: run.updated_at.0,
        }
    }
}

/// Retrieves details for a specific run execution by ID.
///
/// # Errors
/// Returns `StatusCode::NOT_FOUND` if the run does not exist,
/// or `StatusCode::INTERNAL_SERVER_ERROR` if storage query fails.
pub async fn get_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RunResponse>, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let run_id = RunId::from(id);
    let run = SessionStore::get_run(&conn, &run_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "run_not_found",
                format!("Run not found: {run_id}"),
            )
        })?;
    let pending = QuestionStore::list_pending_for_run(&conn, &run_id)
        .map_err(storage_error_to_http)?
        .into_iter()
        .map(QuestionResponse::from)
        .collect();

    Ok(Json(RunResponse::from_run(run, pending)))
}
