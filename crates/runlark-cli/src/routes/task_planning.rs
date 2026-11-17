use crate::repair::WorkflowPipeline;
use crate::routes::error::{ApiError, api_error, storage_error_to_http};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use runlark_domain::{
    BoardCommand, CommandId, DomainEvent, Revision, Task, TaskId, TaskStage, Timestamp,
    UpdateTaskPlanCommand, WorkflowStage,
};
use runlark_local::{
    BoardCommandOutcome, BoardStore, EventStore, SessionStore, StorageError, StoredEvent,
    TaskStore, with_immediate_tx,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct ClarifyTaskRequest {
    pub clarification: String,
    #[serde(default = "default_true")]
    pub advance_to_plan: bool,
}

#[derive(Debug, Deserialize)]
pub struct PlanTaskRequest {
    pub command_id: Option<String>,
    pub plan_text: String,
    pub expected_revision: Option<u64>,
    #[serde(default)]
    pub approve: bool,
}

#[derive(Debug, Serialize)]
pub struct ClarifyTaskResponse {
    pub task: Task,
    pub workflow_stage: WorkflowStage,
}

const fn default_true() -> bool {
    true
}

/// Records clarification notes on a task and optionally advances `Clarification -> Plan`.
///
/// # Errors
/// Returns `404` if task missing, `409` on invalid workflow transition.
pub async fn clarify_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ClarifyTaskRequest>,
) -> Result<Json<ClarifyTaskResponse>, ApiError> {
    if req.clarification.trim().is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_clarification",
            "clarification is required",
        ));
    }

    let task_id = TaskId::from(id);
    let now = Timestamp(AppState::current_timestamp_ms());
    let tx_out = apply_clarification_tx(&state, &task_id, &req, now)?;
    if let Some(ref sid) = tx_out.session_id
        && !tx_out.events.is_empty()
    {
        state.publish_events(sid.as_ref(), &tx_out.events);
    }

    if req.advance_to_plan
        && let Some(ref rid) = tx_out.run_id
    {
        advance_clarification_run(&state, rid, now)?;
    }

    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let updated = TaskStore::get_task(&conn, &task_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "task_not_found",
                format!("Task not found: {task_id}"),
            )
        })?;
    let workflow_stage = updated.workflow_stage;
    Ok(Json(ClarifyTaskResponse {
        task: updated,
        workflow_stage,
    }))
}

struct ClarifyTxOutcome {
    run_id: Option<runlark_domain::RunId>,
    session_id: Option<runlark_domain::SessionId>,
    events: Vec<StoredEvent>,
}

fn apply_clarification_tx(
    state: &AppState,
    task_id: &TaskId,
    req: &ClarifyTaskRequest,
    now: Timestamp,
) -> Result<ClarifyTxOutcome, ApiError> {
    let mut conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    with_immediate_tx::<_, StorageError, _>(&mut conn, |tx| {
        let mut task = TaskStore::get_task(tx, task_id)?
            .ok_or_else(|| StorageError::TaskNotFound(task_id.clone()))?;
        let expected = task.revision;
        let prev_stage = task.stage;
        if task.description.trim().is_empty() {
            task.description.clone_from(&req.clarification);
        } else {
            task.description = format!(
                "{}\n\nClarification: {}",
                task.description, req.clarification
            );
        }
        if req.advance_to_plan && task.workflow_stage == WorkflowStage::Clarification {
            task.workflow_stage = WorkflowStage::Plan;
            if task.stage == TaskStage::Inbox {
                task.stage = TaskStage::Ready;
            }
        }
        task.revision = Revision(expected.0 + 1);
        task.updated_at = now;
        TaskStore::update_task_with_revision(tx, &task, expected)?;

        let events = append_clarify_events(tx, &task, prev_stage, &req.clarification, now)?;
        Ok(ClarifyTxOutcome {
            run_id: task.active_run_id,
            session_id: task.active_session_id,
            events,
        })
    })
    .map_err(storage_error_to_http)
}

fn append_clarify_events(
    tx: &runlark_local::Connection,
    task: &Task,
    prev_stage: TaskStage,
    clarification: &str,
    now: Timestamp,
) -> Result<Vec<StoredEvent>, StorageError> {
    let mut events = Vec::new();
    let stage_changed = task.stage != prev_stage;
    if let Some(ref rid) = task.active_run_id {
        if stage_changed {
            let stage_ev = DomainEvent::TaskStageChanged {
                task_id: task.id.clone(),
                from: prev_stage,
                to: task.stage,
                timestamp: now,
            };
            events.push(EventStore::append_domain_event(tx, rid, &stage_ev, now)?);
        }
        let payload = serde_json::json!({
            "task_id": task.id.0,
            "clarification": clarification,
        })
        .to_string();
        events.push(EventStore::append_wire_event(
            tx,
            rid,
            "TaskClarified",
            &payload,
            now,
        )?);
    } else if stage_changed {
        let stage_ev = DomainEvent::TaskStageChanged {
            task_id: task.id.clone(),
            from: prev_stage,
            to: task.stage,
            timestamp: now,
        };
        let synthetic_run = runlark_domain::RunId::from(format!("board:{}:clarify", task.id.0));
        events.push(EventStore::append_domain_event(
            tx,
            &synthetic_run,
            &stage_ev,
            now,
        )?);
    }
    Ok(events)
}

fn advance_clarification_run(
    state: &AppState,
    rid: &runlark_domain::RunId,
    now: Timestamp,
) -> Result<(), ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let run_stage = SessionStore::get_run(&conn, rid)
        .map_err(storage_error_to_http)?
        .map(|r| r.workflow_stage);
    drop(conn);
    if run_stage == Some(WorkflowStage::Clarification) {
        WorkflowPipeline::advance_stage_with_task(state, rid, WorkflowStage::Plan, now)
            .map_err(|e| api_error(StatusCode::CONFLICT, "transition_conflict", e.to_string()))?;
    }
    Ok(())
}

/// Updates a task's plan (`plan_version += 1`) and optionally approves the plan into `Implementation`.
///
/// # Errors
/// Returns `400` if `plan_text` is empty, `404` if task missing, `409` on revision/command conflict.
pub async fn plan_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<PlanTaskRequest>,
) -> Result<Json<Task>, ApiError> {
    if req.plan_text.trim().is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_plan",
            "plan_text is required",
        ));
    }

    let task_id = TaskId::from(id);
    let now = Timestamp(AppState::current_timestamp_ms());
    let (applied, active_run_id, wf_stage) = {
        let mut conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        let task = TaskStore::get_task(&conn, &task_id)
            .map_err(storage_error_to_http)?
            .ok_or_else(|| {
                api_error(
                    StatusCode::NOT_FOUND,
                    "task_not_found",
                    format!("Task not found: {task_id}"),
                )
            })?;
        let explicit_cmd_id = req.command_id.filter(|s| !s.trim().is_empty());
        let command_id = CommandId::from(
            explicit_cmd_id
                .clone()
                .unwrap_or_else(|| format!("cmd_plan_{}", Uuid::new_v4().simple())),
        );
        let prior_rev = if req.expected_revision.is_none() && explicit_cmd_id.is_some() {
            BoardStore::get_command_payload(&conn, &command_id)
                .map_err(storage_error_to_http)?
                .and_then(|json| serde_json::from_str::<BoardCommand>(&json).ok())
                .and_then(|c| match c {
                    BoardCommand::UpdateTaskPlan(prev) => Some(prev.expected_revision),
                    _ => None,
                })
        } else {
            None
        };
        let expected_revision =
            prior_rev.unwrap_or_else(|| Revision(req.expected_revision.unwrap_or(task.revision.0)));
        let cmd = BoardCommand::UpdateTaskPlan(UpdateTaskPlanCommand {
            command_id,
            task_id: task_id.clone(),
            plan_text: req.plan_text,
            expected_revision,
            created_at: now,
        });
        let outcome =
            BoardStore::apply_board_command(&mut conn, &cmd, now).map_err(storage_error_to_http)?;
        let updated = TaskStore::get_task(&conn, &task_id)
            .map_err(storage_error_to_http)?
            .ok_or_else(|| {
                api_error(
                    StatusCode::NOT_FOUND,
                    "task_not_found",
                    format!("Task not found: {task_id}"),
                )
            })?;
        (
            matches!(outcome, BoardCommandOutcome::Applied { .. }),
            updated.active_run_id,
            updated.workflow_stage,
        )
    };

    if applied && req.approve {
        apply_plan_approval(&state, &task_id, active_run_id.as_ref(), wf_stage, now).await?;
    }

    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let final_task = TaskStore::get_task(&conn, &task_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "task_not_found",
                format!("Task not found: {task_id}"),
            )
        })?;
    Ok(Json(final_task))
}

async fn apply_plan_approval(
    state: &AppState,
    task_id: &TaskId,
    active_run_id: Option<&runlark_domain::RunId>,
    wf_stage: WorkflowStage,
    now: Timestamp,
) -> Result<(), ApiError> {
    if let Some(rid) = active_run_id {
        let run_stage = {
            let conn = state.get_connection().map_err(|e| {
                api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    e.to_string(),
                )
            })?;
            SessionStore::get_run(&conn, rid)
                .map_err(storage_error_to_http)?
                .map(|r| r.workflow_stage)
        };
        if run_stage == Some(WorkflowStage::Plan) {
            WorkflowPipeline::advance_stage_with_task(
                state,
                rid,
                WorkflowStage::Implementation,
                now,
            )
            .map_err(|e| api_error(StatusCode::CONFLICT, "transition_conflict", e.to_string()))?;
        }
    } else if wf_stage == WorkflowStage::Plan {
        let mut conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        with_immediate_tx::<_, StorageError, _>(&mut conn, |tx| {
            if let Some(mut task) = TaskStore::get_task(tx, task_id)? {
                let expected = task.revision;
                let prev_stage = task.stage;
                task.workflow_stage = WorkflowStage::Implementation;
                if task.stage == TaskStage::Inbox {
                    task.stage = TaskStage::Ready;
                }
                task.revision = Revision(expected.0 + 1);
                task.updated_at = now;
                TaskStore::update_task_with_revision(tx, &task, expected)?;
                if task.stage != prev_stage {
                    let ev = DomainEvent::TaskStageChanged {
                        task_id: task.id,
                        from: prev_stage,
                        to: task.stage,
                        timestamp: now,
                    };
                    let synthetic_run =
                        runlark_domain::RunId::from(format!("board:{}:plan", task_id.0));
                    EventStore::append_domain_event(tx, &synthetic_run, &ev, now)?;
                }
            }
            Ok(())
        })
        .map_err(storage_error_to_http)?;
    }
    state.notify_scheduler();
    tokio::task::yield_now().await;
    Ok(())
}
