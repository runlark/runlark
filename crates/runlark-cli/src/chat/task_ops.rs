use super::{ChatDispatchRequest, ChatDispatchResponse};
use crate::routes::board::{BoardQuery, get_board};
use crate::routes::error::{ApiError, api_error, storage_error_to_http};
use crate::routes::task_actions::{
    AcceptTaskRequest, RejectTaskRequest, StartTaskRequest, accept_task, reject_task, start_task,
};
use crate::routes::tasks::{
    CreateTaskRequest, MoveTaskRequest, PlanTaskRequest, create_task, move_task, plan_task,
};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use runlark_domain::{RunId, TaskId, TaskKind, TaskStage};
use runlark_local::{SessionStore, TaskStore};
use serde_json::Value;

pub(super) async fn exec_create_task(
    state: AppState,
    req: ChatDispatchRequest,
    title: String,
    kind: TaskKind,
) -> Result<(StatusCode, ChatDispatchResponse), ApiError> {
    let (status, Json(task)) = create_task(
        State(state),
        Json(CreateTaskRequest {
            command_id: req.command_id,
            task_id: None,
            project_id: req.project_id,
            title,
            description: String::new(),
            kind,
            base_ref: None,
            budget_override: None,
        }),
    )
    .await?;
    let cmd_name = if kind == TaskKind::QuickFix {
        "fix"
    } else {
        "task"
    };
    Ok((
        status,
        ChatDispatchResponse {
            command: cmd_name.to_string(),
            summary: format!("Created task {} ({:?}): {}", task.id, task.kind, task.title),
            data: serde_json::to_value(&task).unwrap_or(Value::Null),
        },
    ))
}

pub(super) async fn exec_move_task(
    state: AppState,
    req: ChatDispatchRequest,
    task_id: &TaskId,
    target_stage: TaskStage,
) -> Result<(StatusCode, ChatDispatchResponse), ApiError> {
    let Json(task) = move_task(
        State(state),
        Path(task_id.0.clone()),
        Json(MoveTaskRequest {
            command_id: req.command_id,
            stage: target_stage,
            actor: None,
            actor_name: req.actor,
            expected_revision: None,
        }),
    )
    .await?;
    Ok((
        StatusCode::OK,
        ChatDispatchResponse {
            command: "move".to_string(),
            summary: format!("Moved task {} to {:?}", task.id, task.stage),
            data: serde_json::to_value(&task).unwrap_or(Value::Null),
        },
    ))
}

pub(super) async fn exec_update_plan(
    state: AppState,
    req: ChatDispatchRequest,
    task_id: &TaskId,
    plan_text: String,
) -> Result<(StatusCode, ChatDispatchResponse), ApiError> {
    let Json(task) = plan_task(
        State(state),
        Path(task_id.0.clone()),
        Json(PlanTaskRequest {
            command_id: req.command_id,
            plan_text,
            expected_revision: None,
            approve: false,
        }),
    )
    .await?;
    Ok((
        StatusCode::OK,
        ChatDispatchResponse {
            command: "plan".to_string(),
            summary: format!("Updated plan for {} (v{})", task.id, task.plan_version),
            data: serde_json::to_value(&task).unwrap_or(Value::Null),
        },
    ))
}

pub(super) async fn exec_start_run(
    state: AppState,
    req: ChatDispatchRequest,
    task_id: &TaskId,
    prompt: Option<String>,
) -> Result<(StatusCode, ChatDispatchResponse), ApiError> {
    let (status, Json(resp)) = start_task(
        State(state),
        Path(task_id.0.clone()),
        Some(Json(StartTaskRequest {
            command_id: req.command_id,
            run_id: req.run_id,
            session_id: req.session_id,
            prompt,
            expected_revision: None,
            runner_id: None,
        })),
    )
    .await?;
    Ok((
        status,
        ChatDispatchResponse {
            command: "run".to_string(),
            summary: format!(
                "Task {} run {} is {}",
                resp.task.id, resp.run.id, resp.run_status
            ),
            data: serde_json::to_value(&resp).unwrap_or(Value::Null),
        },
    ))
}

pub(super) async fn exec_accept_target(
    state: AppState,
    req: ChatDispatchRequest,
    target_id: &str,
    comment: Option<String>,
) -> Result<(StatusCode, ChatDispatchResponse), ApiError> {
    let task_id = resolve_target_task_id(&state, target_id)?;
    let Json(resp) = accept_task(
        State(state),
        Path(task_id.0),
        Some(Json(AcceptTaskRequest {
            command_id: req.command_id,
            actor: req.actor,
            comment,
            ..AcceptTaskRequest::default()
        })),
    )
    .await?;
    Ok((
        StatusCode::OK,
        ChatDispatchResponse {
            command: "accept".to_string(),
            summary: format!("Accepted task {} (run {})", resp.task.id, resp.run.id),
            data: serde_json::to_value(&resp).unwrap_or(Value::Null),
        },
    ))
}

pub(super) async fn exec_reject_target(
    state: AppState,
    req: ChatDispatchRequest,
    target_id: &str,
    reason: String,
) -> Result<(StatusCode, ChatDispatchResponse), ApiError> {
    let task_id = resolve_target_task_id(&state, target_id)?;
    let Json(resp) = reject_task(
        State(state),
        Path(task_id.0),
        Json(RejectTaskRequest {
            command_id: req.command_id,
            actor: req.actor,
            reason,
            expected_revision: None,
        }),
    )
    .await?;
    Ok((
        StatusCode::OK,
        ChatDispatchResponse {
            command: "reject".to_string(),
            summary: format!(
                "Requested changes on task {} (run {})",
                resp.task.id, resp.run.id
            ),
            data: serde_json::to_value(&resp).unwrap_or(Value::Null),
        },
    ))
}

pub(super) async fn exec_board(
    state: AppState,
    project_id: Option<String>,
) -> Result<(StatusCode, ChatDispatchResponse), ApiError> {
    let Json(board) = get_board(State(state), Query(BoardQuery { project_id })).await?;
    Ok((
        StatusCode::OK,
        ChatDispatchResponse {
            command: "board".to_string(),
            summary: format!(
                "Board: {} tasks, {}/{} slots used, {} queued",
                board.tasks.len(),
                board.slots.used,
                board.slots.max,
                board.queued_runs.len()
            ),
            data: serde_json::to_value(&board).unwrap_or(Value::Null),
        },
    ))
}

pub(super) async fn exec_approve_plan(
    state: &AppState,
    task_id: &TaskId,
    comment: Option<String>,
) -> Result<(StatusCode, ChatDispatchResponse), ApiError> {
    let trimmed_comment = comment
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let plan_text = {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        let task = TaskStore::get_task(&conn, task_id)
            .map_err(storage_error_to_http)?
            .ok_or_else(|| {
                api_error(
                    StatusCode::NOT_FOUND,
                    "task_not_found",
                    format!("Task not found: {task_id}"),
                )
            })?;
        match (task.plan_text, trimmed_comment.as_deref()) {
            (Some(existing), Some(note)) => {
                format!("{}\n\nApproval note: {note}", existing.trim_end())
            }
            (Some(existing), None) => existing,
            (None, Some(note)) => note.to_string(),
            (None, None) => format!("Approved plan for {}", task.title),
        }
    };
    let Json(updated) = plan_task(
        State(state.clone()),
        Path(task_id.0.clone()),
        Json(PlanTaskRequest {
            command_id: None,
            plan_text,
            expected_revision: None,
            approve: true,
        }),
    )
    .await?;
    let summary = match trimmed_comment {
        Some(note) => format!(
            "Approved plan for {} (v{}; note: {note})",
            updated.id, updated.plan_version
        ),
        None => format!(
            "Approved plan for {} (v{})",
            updated.id, updated.plan_version
        ),
    };
    Ok((
        StatusCode::OK,
        ChatDispatchResponse {
            command: "plan_approve".to_string(),
            summary,
            data: serde_json::to_value(&updated).unwrap_or(Value::Null),
        },
    ))
}

fn resolve_target_task_id(state: &AppState, target_id: &str) -> Result<TaskId, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    if TaskStore::get_task(&conn, &TaskId::from(target_id))
        .map_err(storage_error_to_http)?
        .is_some()
    {
        return Ok(TaskId::from(target_id));
    }
    if let Some(run) =
        SessionStore::get_run(&conn, &RunId::from(target_id)).map_err(storage_error_to_http)?
        && let Some(tid) = run.task_id
    {
        return Ok(tid);
    }
    Err(api_error(
        StatusCode::NOT_FOUND,
        "target_not_found",
        format!("No task found for `{target_id}`"),
    ))
}
