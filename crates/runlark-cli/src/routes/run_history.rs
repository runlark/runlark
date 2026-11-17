use crate::git_pr::{compute_worktree_diff, store_diff_in_cas};
use crate::routes::error::{ApiError, api_error, deterministic_id, storage_error_to_http};
use crate::routes::events::EventItemResponse;
use crate::routes::questions::QuestionResponse;
use crate::state::AppState;
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use runlark_domain::{
    ArtifactDigest, ArtifactRef, Attempt, AttemptId, AttemptOutcome, BoardCommand, CommandId,
    CostSummary, Decision, RecordAttemptCommand, Revision, Run, RunId, TaskId, Timestamp,
    WorkflowStage,
};
use runlark_local::{
    ArtifactCasStore, AttemptStore, DecisionStore, EventStore, QuestionStore, RunQueries,
    SessionStore,
};
use serde::{Deserialize, Serialize};
use std::path::Path as StdPath;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct ListRunsQuery {
    pub task_id: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ArtifactItem {
    #[serde(flatten)]
    pub artifact: ArtifactRef,
    pub label: String,
}

#[derive(Debug, Serialize)]
pub struct RunHistoryResponse {
    pub run: Run,
    pub attempts: Vec<Attempt>,
    pub cost_summary: CostSummary,
    pub decisions: Vec<Decision>,
    pub artifacts: Vec<ArtifactItem>,
    pub events: Vec<EventItemResponse>,
    pub pending_questions: Vec<QuestionResponse>,
}

#[derive(Debug, Serialize)]
pub struct RunDiffResponse {
    pub run_id: String,
    pub base_sha: String,
    pub head_sha: String,
    pub branch_name: String,
    pub changed_files: Vec<String>,
    pub diff: String,
    pub diff_bytes: u64,
    pub digest_sha256: String,
}

#[derive(Debug, Serialize)]
pub struct RunAttemptsResponse {
    pub run_id: String,
    pub attempts: Vec<Attempt>,
    pub cost_summary: CostSummary,
}

#[derive(Debug, Deserialize)]
pub struct RecordAttemptRequest {
    pub command_id: Option<String>,
    pub attempt_id: Option<String>,
    #[serde(default)]
    pub workflow_stage: WorkflowStage,
    #[serde(default = "default_succeeded")]
    pub outcome: AttemptOutcome,
    pub cost_micro_usd: Option<u64>,
    pub unknown_cost_reason: Option<String>,
    pub repair_reason: Option<String>,
    pub expected_revision: Option<u64>,
}

fn default_succeeded() -> AttemptOutcome {
    AttemptOutcome::Succeeded
}

#[derive(Debug, Deserialize)]
pub struct ArtifactQuery {
    #[serde(default)]
    pub meta: bool,
}

/// Lists runs, optionally filtered by `task_id` or `status`.
///
/// # Errors
/// Returns `500` on storage failure.
pub async fn list_runs(
    State(state): State<AppState>,
    Query(query): Query<ListRunsQuery>,
) -> Result<Json<Vec<Run>>, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let mut runs = match query.task_id.filter(|s| !s.trim().is_empty()) {
        Some(tid) => RunQueries::list_runs_for_task(&conn, &TaskId::from(tid))
            .map_err(storage_error_to_http)?,
        None => RunQueries::list_runs(&conn).map_err(storage_error_to_http)?,
    };
    if let Some(status_filter) = query.status.filter(|s| !s.trim().is_empty()) {
        runs.retain(|r| {
            crate::routes::error::run_status_str(r.status).eq_ignore_ascii_case(&status_filter)
        });
    }
    Ok(Json(runs))
}

/// Retrieves full execution history for a run (attempts, cost summary, decisions, artifacts,
/// events, and pending questions).
///
/// # Errors
/// Returns `404` if the run does not exist, or `500` on storage failure.
pub async fn get_run_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RunHistoryResponse>, ApiError> {
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
    let attempts =
        AttemptStore::list_attempts_for_run(&conn, &run_id).map_err(storage_error_to_http)?;
    let cost_summary =
        AttemptStore::cost_summary_for_run(&conn, &run_id, run.cost_summary.held_micro_usd)
            .map_err(storage_error_to_http)?;
    let decisions =
        DecisionStore::list_decisions_for_run(&conn, &run_id).map_err(storage_error_to_http)?;
    let artifacts = ArtifactCasStore::list_artifacts_for_run(&conn, &run_id)
        .map_err(storage_error_to_http)?
        .into_iter()
        .map(|(artifact, label)| ArtifactItem { artifact, label })
        .collect();
    let events = EventStore::get_events(&conn, &run_id, None)
        .map_err(storage_error_to_http)?
        .into_iter()
        .map(EventItemResponse::from_stored)
        .collect();
    let pending_questions = QuestionStore::list_pending_for_run(&conn, &run_id)
        .map_err(storage_error_to_http)?
        .into_iter()
        .map(QuestionResponse::from)
        .collect();

    Ok(Json(RunHistoryResponse {
        run,
        attempts,
        cost_summary,
        decisions,
        artifacts,
        events,
        pending_questions,
    }))
}

/// Computes the live git diff in the run's isolated worktree and stores it in CAS.
///
/// # Errors
/// Returns `404` if run missing, `409` if worktree is not a git repo, or `500` on CAS/storage failure.
pub async fn get_run_diff(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RunDiffResponse>, ApiError> {
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

    let wt = run.worktree_path.clone();
    let base_hint = run.base_sha.clone();
    let snap = tokio::task::spawn_blocking(move || {
        compute_worktree_diff(StdPath::new(&wt), base_hint.as_deref())
    })
    .await
    .map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "diff_panic",
            e.to_string(),
        )
    })?
    .map_err(|e| api_error(StatusCode::CONFLICT, "git_diff_failed", e.to_string()))?;

    let cas = state.cas_store();
    let now = Timestamp(AppState::current_timestamp_ms());
    let artifact =
        store_diff_in_cas(&cas, &conn, &run_id, &snap, now).map_err(storage_error_to_http)?;

    Ok(Json(RunDiffResponse {
        run_id: run_id.0,
        base_sha: snap.base_sha,
        head_sha: snap.head_sha,
        branch_name: snap.branch_name,
        changed_files: snap.changed_files,
        diff: snap.diff_text,
        diff_bytes: snap.diff_bytes,
        digest_sha256: artifact.digest_sha256.to_string(),
    }))
}

/// Lists all recorded attempts and explicit [`CostSummary`] for a run.
///
/// # Errors
/// Returns `404` if run missing, or `500` on storage failure.
pub async fn list_run_attempts(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RunAttemptsResponse>, ApiError> {
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
    let attempts =
        AttemptStore::list_attempts_for_run(&conn, &run_id).map_err(storage_error_to_http)?;
    let cost_summary =
        AttemptStore::cost_summary_for_run(&conn, &run_id, run.cost_summary.held_micro_usd)
            .map_err(storage_error_to_http)?;
    Ok(Json(RunAttemptsResponse {
        run_id: run_id.0,
        attempts,
        cost_summary,
    }))
}

/// Records an execution/repair attempt and cost delta (`None` = explicit `UnknownCost`).
///
/// # Errors
/// Returns `404` if run missing, `409` on revision/command conflict, or `500` on storage failure.
pub async fn record_run_attempt(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<RecordAttemptRequest>,
) -> Result<(StatusCode, Json<RunAttemptsResponse>), ApiError> {
    let mut conn = state.get_connection().map_err(|e| {
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

    let explicit_cmd_id = req.command_id.filter(|s| !s.trim().is_empty());
    let command_id = CommandId::from(
        explicit_cmd_id
            .clone()
            .unwrap_or_else(|| format!("cmd_att_{}", Uuid::new_v4().simple())),
    );
    let attempt_id = AttemptId::from(
        req.attempt_id
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| match &explicit_cmd_id {
                Some(cid) => deterministic_id("att", cid),
                None => format!("att_{}", Uuid::new_v4().simple()),
            }),
    );
    let prior_rev = if req.expected_revision.is_none() && explicit_cmd_id.is_some() {
        runlark_local::BoardStore::get_command_payload(&conn, &command_id)
            .map_err(storage_error_to_http)?
            .and_then(|json| serde_json::from_str::<BoardCommand>(&json).ok())
            .and_then(|c| match c {
                BoardCommand::RecordAttempt(prev) => Some(prev.expected_revision),
                _ => None,
            })
    } else {
        None
    };
    let expected_revision =
        prior_rev.unwrap_or_else(|| Revision(req.expected_revision.unwrap_or(run.revision.0)));
    let now = Timestamp(AppState::current_timestamp_ms());
    let cmd = BoardCommand::RecordAttempt(RecordAttemptCommand {
        command_id,
        attempt_id,
        run_id: run_id.clone(),
        workflow_stage: req.workflow_stage,
        outcome: req.outcome,
        cost_micro_usd: req.cost_micro_usd,
        unknown_cost_reason: req.unknown_cost_reason,
        repair_reason: req.repair_reason,
        expected_revision,
        created_at: now,
    });
    let outcome = runlark_local::BoardStore::apply_board_command(&mut conn, &cmd, now);
    let applied = outcome.map_err(storage_error_to_http)?;
    let status = match applied {
        runlark_local::BoardCommandOutcome::Applied { .. } => StatusCode::CREATED,
        runlark_local::BoardCommandOutcome::AlreadyExists { .. } => StatusCode::OK,
    };

    let updated_run = SessionStore::get_run(&conn, &run_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "run_not_found",
                format!("Run not found: {run_id}"),
            )
        })?;
    let attempts =
        AttemptStore::list_attempts_for_run(&conn, &run_id).map_err(storage_error_to_http)?;
    let cost_summary =
        AttemptStore::cost_summary_for_run(&conn, &run_id, updated_run.cost_summary.held_micro_usd)
            .map_err(storage_error_to_http)?;
    Ok((
        status,
        Json(RunAttemptsResponse {
            run_id: run_id.0,
            attempts,
            cost_summary,
        }),
    ))
}

/// Fetches and SHA-256 verifies an immutable CAS artifact by digest (raw bytes, or JSON metadata
/// when `?meta=true`).
///
/// # Errors
/// Returns `400` on invalid SHA-256 digest, `404` if missing, or `500` on CAS corruption.
pub async fn get_artifact(
    State(state): State<AppState>,
    Path(digest_raw): Path<String>,
    Query(query): Query<ArtifactQuery>,
) -> Result<Response, ApiError> {
    let digest = ArtifactDigest::parse(digest_raw)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, "invalid_digest", e.to_string()))?;
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let meta = ArtifactCasStore::get_artifact_meta(&conn, &digest)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "artifact_not_found",
                format!("Artifact not found: {digest}"),
            )
        })?;

    if query.meta {
        let json = serde_json::to_vec(&meta).map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "serialization_error",
                e.to_string(),
            )
        })?;
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json))
            .map_err(|e| {
                api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "response_error",
                    e.to_string(),
                )
            });
    }

    let bytes = state
        .cas_store()
        .get_bytes(&digest)
        .map_err(storage_error_to_http)?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, meta.media_type)
        .body(Body::from(bytes))
        .map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "response_error",
                e.to_string(),
            )
        })
}
