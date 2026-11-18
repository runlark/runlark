use crate::routes::error::{ApiError, api_error, storage_error_to_http};
use crate::routes::projects::ProjectResponse;
use crate::routes::questions::QuestionResponse;
use crate::scheduler::{DEFAULT_MAX_CONCURRENT_SLOTS, Scheduler, SchedulerStatus};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use runlark_domain::{
    ArtifactRef, Attempt, CostSummary, Decision, OfflineDraft, ProjectId, ProjectOwnershipMode,
    Run, RunOwnership, RunnerRegistration, Task, TaskComment, TaskStage, WorkspaceMember,
};
use runlark_local::{
    AttemptStore, BoardProjection, CommentStore, ProjectStore, QuestionStore, RunQueries,
    SessionStore, SyncStore,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct BoardQuery {
    pub project_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BoardArtifactItem {
    #[serde(flatten)]
    pub artifact: ArtifactRef,
    pub label: String,
}

#[derive(Debug, Serialize)]
pub struct BoardTaskEntry {
    pub task: Task,
    pub latest_run: Option<Run>,
    pub latest_run_ownership: Option<RunOwnership>,
    pub session_revision: Option<u64>,
    pub attempts: Vec<Attempt>,
    pub cost_summary: CostSummary,
    pub decisions: Vec<Decision>,
    pub comments: Vec<TaskComment>,
    pub artifacts: Vec<BoardArtifactItem>,
    pub pending_questions: Vec<QuestionResponse>,
}

#[derive(Debug, Serialize)]
pub struct BoardColumns {
    pub inbox: Vec<BoardTaskEntry>,
    pub ready: Vec<BoardTaskEntry>,
    pub in_progress: Vec<BoardTaskEntry>,
    pub acceptance: Vec<BoardTaskEntry>,
    pub done: Vec<BoardTaskEntry>,
}

#[derive(Debug, Serialize)]
pub struct SlotSummary {
    pub max: u64,
    pub used: u64,
    pub available: u64,
}

#[derive(Debug, Serialize)]
pub struct BoardResponse {
    pub project: Option<ProjectResponse>,
    pub ownership_mode: ProjectOwnershipMode,
    pub last_cursor_seq: u64,
    pub columns: BoardColumns,
    pub tasks: Vec<BoardTaskEntry>,
    pub active_runs: Vec<Run>,
    pub queued_runs: Vec<Run>,
    pub attempts: Vec<Attempt>,
    pub cost_summary: CostSummary,
    pub decisions: Vec<Decision>,
    pub pending_questions: Vec<QuestionResponse>,
    pub pending_drafts: Vec<OfflineDraft>,
    pub runners: Vec<RunnerRegistration>,
    pub members: Vec<WorkspaceMember>,
    pub run_ownerships: HashMap<String, RunOwnership>,
    pub slots: SlotSummary,
}

/// Projects the full Kanban board state for a project (or the first registered project).
///
/// # Errors
/// Returns `404 Not Found` if an explicit `project_id` does not exist, or `500` on storage failure.
pub async fn get_board(
    State(state): State<AppState>,
    Query(query): Query<BoardQuery>,
) -> Result<Json<BoardResponse>, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;

    let explicit_pid = query
        .project_id
        .filter(|s| !s.trim().is_empty())
        .map(ProjectId::from);
    let resolved_pid = if let Some(pid) = explicit_pid {
        if ProjectStore::get_project(&conn, &pid)
            .map_err(storage_error_to_http)?
            .is_none()
        {
            return Err(api_error(
                StatusCode::NOT_FOUND,
                "project_not_found",
                format!("Project not found: {pid}"),
            ));
        }
        Some(pid)
    } else {
        RunQueries::resolve_default_project_id(&conn, None).map_err(storage_error_to_http)?
    };

    let Some(project_id) = resolved_pid else {
        return Ok(Json(empty_board_response(&conn)?));
    };

    let snapshot =
        BoardProjection::project_board(&conn, &project_id).map_err(storage_error_to_http)?;
    Ok(Json(build_project_board_response(&conn, snapshot)?))
}

fn empty_board_response(conn: &runlark_local::Connection) -> Result<BoardResponse, ApiError> {
    let active_runs = RunQueries::list_executing_runs(conn).map_err(storage_error_to_http)?;
    let queued_runs =
        BoardProjection::list_queued_runs_fifo(conn).map_err(storage_error_to_http)?;
    let used = u64::try_from(active_runs.len()).unwrap_or(u64::MAX);
    Ok(BoardResponse {
        project: None,
        ownership_mode: ProjectOwnershipMode::SoloLocal,
        last_cursor_seq: 0,
        columns: BoardColumns {
            inbox: Vec::new(),
            ready: Vec::new(),
            in_progress: Vec::new(),
            acceptance: Vec::new(),
            done: Vec::new(),
        },
        tasks: Vec::new(),
        active_runs,
        queued_runs,
        attempts: Vec::new(),
        cost_summary: CostSummary::default(),
        decisions: Vec::new(),
        pending_questions: Vec::new(),
        pending_drafts: Vec::new(),
        runners: Vec::new(),
        members: Vec::new(),
        run_ownerships: HashMap::new(),
        slots: SlotSummary {
            max: DEFAULT_MAX_CONCURRENT_SLOTS,
            used,
            available: DEFAULT_MAX_CONCURRENT_SLOTS.saturating_sub(used),
        },
    })
}

fn build_project_board_response(
    conn: &runlark_local::Connection,
    snapshot: runlark_local::BoardSnapshot,
) -> Result<BoardResponse, ApiError> {
    let project_id = snapshot.project.id.clone();
    let max_slots =
        u64::from(snapshot.project.max_concurrent_runs).min(DEFAULT_MAX_CONCURRENT_SLOTS);
    let active_runs = RunQueries::list_executing_runs(conn)
        .map_err(storage_error_to_http)?
        .into_iter()
        .filter(|r| r.project_id.as_ref() == Some(&project_id))
        .collect::<Vec<_>>();
    let all_pending_questions: Vec<QuestionResponse> = QuestionStore::list_pending(conn)
        .map_err(storage_error_to_http)?
        .into_iter()
        .map(QuestionResponse::from)
        .collect();

    let mut entries = Vec::with_capacity(snapshot.tasks.len());
    let mut all_attempts = Vec::new();
    let mut all_decisions = Vec::new();
    let mut total_held = 0_u64;
    let mut columns = BoardColumns {
        inbox: Vec::new(),
        ready: Vec::new(),
        in_progress: Vec::new(),
        acceptance: Vec::new(),
        done: Vec::new(),
    };

    for summary in snapshot.tasks {
        let attempts = if let Some(ref run) = summary.latest_run {
            total_held = total_held.saturating_add(run.cost_summary.held_micro_usd);
            AttemptStore::list_attempts_for_run(conn, &run.id).map_err(storage_error_to_http)?
        } else {
            Vec::new()
        };
        let task_questions: Vec<QuestionResponse> = match &summary.latest_run {
            Some(run) => QuestionStore::list_pending_for_run(conn, &run.id)
                .map_err(storage_error_to_http)?
                .into_iter()
                .map(QuestionResponse::from)
                .collect(),
            None => Vec::new(),
        };
        all_attempts.extend(attempts.iter().cloned());
        all_decisions.extend(summary.decisions.iter().cloned());
        let artifacts: Vec<BoardArtifactItem> = summary
            .artifacts
            .into_iter()
            .map(|(artifact, label)| BoardArtifactItem { artifact, label })
            .collect();
        let session_revision =
            resolve_session_revision(conn, &summary.task, summary.latest_run.as_ref())?;
        let entry = BoardTaskEntry {
            task: summary.task,
            latest_run: summary.latest_run,
            latest_run_ownership: summary.latest_run_ownership,
            session_revision,
            attempts,
            cost_summary: summary.cost_summary,
            decisions: summary.decisions,
            comments: summary.comments,
            artifacts,
            pending_questions: task_questions,
        };
        push_column_entry(&mut columns, &entry);
        entries.push(entry);
    }

    let cost_summary = CostSummary::from_attempts(&all_attempts, total_held);
    let used = snapshot.executing_slots_used;
    let project_resp = ProjectResponse::from_project_with_binding(conn, snapshot.project)?;
    let run_ownerships = snapshot
        .run_ownerships
        .into_iter()
        .map(|(k, v)| (k.0, v))
        .collect();
    Ok(BoardResponse {
        project: Some(project_resp),
        ownership_mode: snapshot.ownership_mode,
        last_cursor_seq: snapshot.last_cursor_seq.0,
        columns,
        tasks: entries,
        active_runs,
        queued_runs: snapshot.queued_runs,
        attempts: all_attempts,
        cost_summary,
        decisions: all_decisions,
        pending_questions: all_pending_questions,
        pending_drafts: snapshot.pending_drafts,
        runners: snapshot.runners,
        members: snapshot.members,
        run_ownerships,
        slots: SlotSummary {
            max: max_slots,
            used,
            available: max_slots.saturating_sub(used),
        },
    })
}

fn push_column_entry(columns: &mut BoardColumns, entry: &BoardTaskEntry) {
    let copy = BoardTaskEntry {
        task: entry.task.clone(),
        latest_run: entry.latest_run.clone(),
        latest_run_ownership: entry.latest_run_ownership.clone(),
        session_revision: entry.session_revision,
        attempts: entry.attempts.clone(),
        cost_summary: entry.cost_summary.clone(),
        decisions: entry.decisions.clone(),
        comments: entry.comments.clone(),
        artifacts: entry
            .artifacts
            .iter()
            .map(|a| BoardArtifactItem {
                artifact: a.artifact.clone(),
                label: a.label.clone(),
            })
            .collect(),
        pending_questions: entry.pending_questions.clone(),
    };
    match entry.task.stage {
        TaskStage::Inbox => columns.inbox.push(copy),
        TaskStage::Ready => columns.ready.push(copy),
        TaskStage::InProgress => columns.in_progress.push(copy),
        TaskStage::Acceptance => columns.acceptance.push(copy),
        TaskStage::Done => columns.done.push(copy),
    }
}

/// Returns the current 2-slot scheduler status (`max_slots`, `active_slots_used`, `queued_run_ids`).
///
/// # Errors
/// Returns `500 Internal Server Error` if querying `SQLite` fails.
pub async fn get_scheduler_status(
    State(state): State<AppState>,
) -> Result<Json<SchedulerStatus>, ApiError> {
    let scheduler = Scheduler::new(state);
    let status = scheduler.status().map_err(storage_error_to_http)?;
    Ok(Json(status))
}

/// Helper used by task detail routes to enrich a single task summary.
///
/// # Errors
/// Returns [`ApiError`] if loading task attempts, questions, or runs fails.
pub fn load_enriched_task(
    conn: &runlark_local::Connection,
    task: &Task,
) -> Result<BoardTaskEntry, ApiError> {
    let summary = BoardProjection::task_summary(conn, &task.id).map_err(storage_error_to_http)?;
    let (attempts, pending_questions) = match &summary.latest_run {
        Some(run) => {
            let atts = AttemptStore::list_attempts_for_run(conn, &run.id)
                .map_err(storage_error_to_http)?;
            let qs = QuestionStore::list_pending_for_run(conn, &run.id)
                .map_err(storage_error_to_http)?
                .into_iter()
                .map(QuestionResponse::from)
                .collect();
            (atts, qs)
        }
        None => (Vec::new(), Vec::new()),
    };
    let (latest_run, latest_run_ownership) = match &summary.latest_run {
        Some(r) => (
            SessionStore::get_run(conn, &r.id).map_err(storage_error_to_http)?,
            SyncStore::get_run_ownership(conn, &r.id).map_err(storage_error_to_http)?,
        ),
        None => (None, None),
    };
    let comments = CommentStore::list_comments_for_task(conn, &summary.task.id)
        .map_err(storage_error_to_http)?;
    let session_revision = resolve_session_revision(conn, &summary.task, latest_run.as_ref())?;
    Ok(BoardTaskEntry {
        task: summary.task,
        latest_run,
        latest_run_ownership,
        session_revision,
        attempts,
        cost_summary: summary.cost_summary,
        decisions: summary.decisions,
        comments,
        artifacts: summary
            .artifacts
            .into_iter()
            .map(|(artifact, label)| BoardArtifactItem { artifact, label })
            .collect(),
        pending_questions,
    })
}

fn resolve_session_revision(
    conn: &runlark_local::Connection,
    task: &Task,
    latest_run: Option<&Run>,
) -> Result<Option<u64>, ApiError> {
    let session_id = latest_run
        .map(|r| &r.session_id)
        .or(task.active_session_id.as_ref());
    let Some(sid) = session_id else {
        return Ok(None);
    };
    Ok(SessionStore::get_session(conn, sid)
        .map_err(storage_error_to_http)?
        .map(|s| s.current_revision.0))
}
