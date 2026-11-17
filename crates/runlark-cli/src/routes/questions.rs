use crate::routes::error::{ApiError, api_error, run_status_str, storage_error_to_http};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use runlark_domain::{
    AnswerQuestionCommand, CommandId, Question, QuestionId, QuestionStatus, Revision, RunId,
    SessionId, Timestamp,
};
use runlark_local::{CommandApplyOutcome, LocalStore, QuestionStore, SessionStore};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct ListQuestionsQuery {
    pub session_id: Option<String>,
    pub run_id: Option<String>,
}

#[derive(Deserialize)]
pub struct AnswerQuestionRequest {
    pub command_id: Option<String>,
    pub answer: String,
    pub expected_revision: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct QuestionResponse {
    pub question_id: String,
    pub session_id: String,
    pub run_id: String,
    pub prompt: String,
    pub options: Vec<String>,
    pub status: String,
    pub answer: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Serialize)]
pub struct AnswerQuestionResponse {
    pub command_id: String,
    pub status: String,
    pub question_id: String,
    pub current_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_status: Option<String>,
}

fn question_status_str(status: QuestionStatus) -> &'static str {
    match status {
        QuestionStatus::Pending => "pending",
        QuestionStatus::Answered => "answered",
        QuestionStatus::Cancelled => "cancelled",
    }
}

impl From<Question> for QuestionResponse {
    fn from(q: Question) -> Self {
        Self {
            question_id: q.id.0,
            session_id: q.session_id.0,
            run_id: q.run_id.0,
            prompt: q.prompt,
            options: q.options,
            status: question_status_str(q.status).to_string(),
            answer: q.answer,
            created_at: q.created_at.0,
            updated_at: q.updated_at.0,
        }
    }
}

/// Lists pending human questions, optionally filtered by `session_id` or `run_id`.
///
/// # Errors
/// Returns `StatusCode::INTERNAL_SERVER_ERROR` if querying `SQLite` fails.
pub async fn list_questions(
    State(state): State<AppState>,
    Query(query): Query<ListQuestionsQuery>,
) -> Result<Json<Vec<QuestionResponse>>, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;

    let questions = if let Some(run_id) = query.run_id {
        QuestionStore::list_pending_for_run(&conn, &RunId::from(run_id))
            .map_err(storage_error_to_http)?
    } else if let Some(session_id) = query.session_id {
        QuestionStore::list_pending_for_session(&conn, &SessionId::from(session_id))
            .map_err(storage_error_to_http)?
    } else {
        QuestionStore::list_pending(&conn).map_err(storage_error_to_http)?
    };

    Ok(Json(
        questions.into_iter().map(QuestionResponse::from).collect(),
    ))
}

/// Lists pending human questions for a specific session.
///
/// # Errors
/// Returns `StatusCode::NOT_FOUND` if the session does not exist,
/// or `StatusCode::INTERNAL_SERVER_ERROR` if querying `SQLite` fails.
pub async fn list_session_questions(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<QuestionResponse>>, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;

    let session_id = SessionId::from(id);
    if SessionStore::get_session(&conn, &session_id)
        .map_err(storage_error_to_http)?
        .is_none()
    {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "session_not_found",
            format!("Session not found: {session_id}"),
        ));
    }

    let questions = QuestionStore::list_pending_for_session(&conn, &session_id)
        .map_err(storage_error_to_http)?;
    Ok(Json(
        questions.into_iter().map(QuestionResponse::from).collect(),
    ))
}

/// Answers a pending human question via [`AnswerQuestionCommand`], returning the run to `Running`.
///
/// # Errors
/// Returns `StatusCode::NOT_FOUND` if the question or session does not exist,
/// `StatusCode::CONFLICT` on revision or command payload conflict,
/// or `StatusCode::UNPROCESSABLE_ENTITY` if the question/run is not waiting for approval.
pub async fn answer_question(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<AnswerQuestionRequest>,
) -> Result<(StatusCode, Json<AnswerQuestionResponse>), ApiError> {
    let mut conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;

    let question_id = QuestionId::from(id);
    let question = QuestionStore::get_question(&conn, &question_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "question_not_found",
                format!("Question not found: {question_id}"),
            )
        })?;

    let session = SessionStore::get_session(&conn, &question.session_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "session_not_found",
                format!("Session not found: {}", question.session_id),
            )
        })?;

    let now = AppState::current_timestamp_ms();
    let command_id = CommandId::from(req.command_id.unwrap_or_else(|| Uuid::new_v4().to_string()));
    let expected_revision = Revision(req.expected_revision.unwrap_or(session.current_revision.0));

    let cmd = AnswerQuestionCommand {
        command_id: command_id.clone(),
        session_id: question.session_id,
        run_id: question.run_id,
        question_id: question_id.clone(),
        answer: req.answer,
        expected_revision,
        created_at: Timestamp(now),
    };

    let outcome = LocalStore::answer_question(&mut conn, cmd, Timestamp(now))
        .map_err(storage_error_to_http)?;

    match outcome {
        CommandApplyOutcome::Applied {
            status,
            session,
            run,
            events,
        } => {
            state.publish_events(&session.id.0, &events);
            state.notify_outbox();
            state.notify_dsh_subscriber();
            Ok((
                StatusCode::CREATED,
                Json(AnswerQuestionResponse {
                    command_id: command_id.0,
                    status,
                    question_id: question_id.0,
                    current_revision: session.current_revision.0,
                    run_status: run.as_ref().map(|r| run_status_str(r.status).to_string()),
                }),
            ))
        }
        CommandApplyOutcome::AlreadyExists {
            status,
            session,
            run,
        } => Ok((
            StatusCode::OK,
            Json(AnswerQuestionResponse {
                command_id: command_id.0,
                status,
                question_id: question_id.0,
                current_revision: session.current_revision.0,
                run_status: run.as_ref().map(|r| run_status_str(r.status).to_string()),
            }),
        )),
    }
}
