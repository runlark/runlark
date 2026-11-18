use super::{ChatDispatchRequest, ChatDispatchResponse};
use crate::routes::board::load_enriched_task;
use crate::routes::error::{
    ApiError, api_error, deterministic_id, run_status_str, storage_error_to_http,
};
use crate::routes::team_gate;
use crate::scheduler::Scheduler;
use crate::state::AppState;
use axum::http::StatusCode;
use runlark_domain::{
    CommandId, ProjectId, RunId, SendPromptCommand, SessionCommand, SessionId, TaskId, Timestamp,
    WorkflowStage,
};
use runlark_local::{
    AttemptStore, CommandApplyOutcome, LocalStore, ProjectStore, RunQueries, SessionStore,
    TaskStore,
};
use serde_json::{Value, json};
use uuid::Uuid;

pub(super) fn exec_status(
    state: &AppState,
    target_id: Option<String>,
) -> Result<(StatusCode, ChatDispatchResponse), ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;

    if let Some(target) = target_id {
        if let Some(run) = SessionStore::get_run(&conn, &RunId::from(target.as_str()))
            .map_err(storage_error_to_http)?
        {
            let attempts = AttemptStore::list_attempts_for_run(&conn, &run.id)
                .map_err(storage_error_to_http)?;
            let cost =
                AttemptStore::cost_summary_for_run(&conn, &run.id, run.cost_summary.held_micro_usd)
                    .map_err(storage_error_to_http)?;
            return Ok((
                StatusCode::OK,
                ChatDispatchResponse {
                    command: "status".to_string(),
                    summary: format!(
                        "Run {} is {} ({:?})",
                        run.id,
                        run_status_str(run.status),
                        run.workflow_stage
                    ),
                    data: json!({ "run": run, "attempts": attempts, "cost_summary": cost }),
                },
            ));
        }
        if let Some(task) = TaskStore::get_task(&conn, &TaskId::from(target.as_str()))
            .map_err(storage_error_to_http)?
        {
            let summary = load_enriched_task(&conn, &task)?;
            return Ok((
                StatusCode::OK,
                ChatDispatchResponse {
                    command: "status".to_string(),
                    summary: format!(
                        "Task {} is {:?} ({:?})",
                        summary.task.id, summary.task.stage, summary.task.workflow_stage
                    ),
                    data: serde_json::to_value(&summary).unwrap_or(Value::Null),
                },
            ));
        }
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "target_not_found",
            format!("No run or task found for `{target}`"),
        ));
    }

    let sched = Scheduler::new(state.clone())
        .status()
        .map_err(storage_error_to_http)?;
    Ok((
        StatusCode::OK,
        ChatDispatchResponse {
            command: "status".to_string(),
            summary: format!(
                "Slots: {}/{} active, {} queued",
                sched.active_slots_used,
                sched.max_slots,
                sched.queued_run_ids.len()
            ),
            data: serde_json::to_value(&sched).unwrap_or(Value::Null),
        },
    ))
}

pub(super) fn exec_budget(
    state: &AppState,
    target_id: Option<String>,
) -> Result<(StatusCode, ChatDispatchResponse), ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    if let Some(target) = target_id {
        if let Some(run) = SessionStore::get_run(&conn, &RunId::from(target.as_str()))
            .map_err(storage_error_to_http)?
        {
            let cost =
                AttemptStore::cost_summary_for_run(&conn, &run.id, run.cost_summary.held_micro_usd)
                    .map_err(storage_error_to_http)?;
            return Ok((
                StatusCode::OK,
                ChatDispatchResponse {
                    command: "budget".to_string(),
                    summary: format!(
                        "Run {} budget: known={} micro-USD, unknown_attempts={}",
                        run.id, cost.known_micro_usd, cost.unknown_attempts
                    ),
                    data: json!({
                        "run_id": run.id,
                        "budget_policy": run.budget_policy,
                        "cost_summary": cost,
                    }),
                },
            ));
        }
        if let Some(task) = TaskStore::get_task(&conn, &TaskId::from(target.as_str()))
            .map_err(storage_error_to_http)?
        {
            let enriched = load_enriched_task(&conn, &task)?;
            return Ok((
                StatusCode::OK,
                ChatDispatchResponse {
                    command: "budget".to_string(),
                    summary: format!(
                        "Task {} budget: known={} micro-USD, unknown_attempts={}",
                        enriched.task.id,
                        enriched.cost_summary.known_micro_usd,
                        enriched.cost_summary.unknown_attempts
                    ),
                    data: json!({
                        "task_id": enriched.task.id,
                        "cost_summary": enriched.cost_summary,
                    }),
                },
            ));
        }
        if let Some(proj) = ProjectStore::get_project(&conn, &ProjectId::from(target.as_str()))
            .map_err(storage_error_to_http)?
        {
            return Ok((
                StatusCode::OK,
                ChatDispatchResponse {
                    command: "budget".to_string(),
                    summary: format!(
                        "Project {} policy: max_attempts={}, max_spend={} micro-USD",
                        proj.id,
                        proj.budget_policy.max_attempts,
                        proj.budget_policy.max_spend_micro_usd
                    ),
                    data: json!({
                        "project_id": proj.id,
                        "budget_policy": proj.budget_policy,
                    }),
                },
            ));
        }
    }

    let projects = ProjectStore::list_projects(&conn).map_err(storage_error_to_http)?;
    let runs = RunQueries::list_runs(&conn).map_err(storage_error_to_http)?;
    let mut known = 0_u64;
    let mut unknown = 0_u32;
    for r in &runs {
        let cs = AttemptStore::cost_summary_for_run(&conn, &r.id, r.cost_summary.held_micro_usd)
            .map_err(storage_error_to_http)?;
        known = known.saturating_add(cs.known_micro_usd);
        unknown = unknown.saturating_add(cs.unknown_attempts);
    }
    Ok((
        StatusCode::OK,
        ChatDispatchResponse {
            command: "budget".to_string(),
            summary: format!(
                "Workspace spend: known={known} micro-USD, unknown_attempts={unknown}"
            ),
            data: json!({
                "projects": projects,
                "known_micro_usd": known,
                "unknown_attempts": unknown,
            }),
        },
    ))
}

fn resolve_prompt_session(
    state: &AppState,
    req: &ChatDispatchRequest,
) -> Result<runlark_domain::Session, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let sid_opt = req
        .session_id
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| SessionId::from(s.as_str()));
    if let Some(sid) = sid_opt {
        return SessionStore::get_session(&conn, &sid)
            .map_err(storage_error_to_http)?
            .ok_or_else(|| {
                api_error(
                    StatusCode::NOT_FOUND,
                    "session_not_found",
                    format!("Session not found: {sid}"),
                )
            });
    }
    let Some(rid) = req.run_id.as_deref().filter(|s| !s.trim().is_empty()) else {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "session_required",
            "session_id or run_id is required to send plain-text prompts",
        ));
    };
    let run = SessionStore::get_run(&conn, &RunId::from(rid))
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "run_not_found",
                format!("Run not found: {rid}"),
            )
        })?;
    SessionStore::get_session(&conn, &run.session_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "session_not_found",
                format!("Session not found: {}", run.session_id),
            )
        })
}

pub(super) async fn exec_prompt(
    state: &AppState,
    req: &ChatDispatchRequest,
    text: String,
) -> Result<(StatusCode, ChatDispatchResponse), ApiError> {
    let session = resolve_prompt_session(state, req)?;
    let explicit_cmd_id = req.command_id.as_deref().filter(|s| !s.trim().is_empty());
    let run_id = req
        .run_id
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(RunId::from)
        .or_else(|| session.active_run_id.clone())
        .unwrap_or_else(|| match explicit_cmd_id {
            Some(cid) => RunId::from(deterministic_id("run", cid)),
            None => RunId::from(format!("run_{}", Uuid::new_v4().simple())),
        });
    let now = Timestamp(AppState::current_timestamp_ms());
    let cmd = SessionCommand::SendPrompt(SendPromptCommand {
        command_id: CommandId::from(explicit_cmd_id.map_or_else(
            || format!("cmd_chat_{}", Uuid::new_v4().simple()),
            str::to_string,
        )),
        session_id: session.id.clone(),
        run_id: run_id.clone(),
        prompt: text,
        expected_revision: session.current_revision,
        created_at: now,
    });

    if let Some(team) = team_gate::resolve_session_command_team_context(state, &cmd)?
        && let Some((status, axum::Json(resp))) =
            crate::routes::commands::submit_team_session_command(state, &cmd, team, now).await?
    {
        let created_run_id = resp.run_id.unwrap_or_else(|| run_id.0.clone());
        let workflow_stage = state
            .get_connection()
            .ok()
            .and_then(|conn| {
                SessionStore::get_run(&conn, &RunId::from(created_run_id.as_str()))
                    .ok()
                    .flatten()
            })
            .map_or(WorkflowStage::Implementation, |r| r.workflow_stage);
        return Ok((
            status,
            ChatDispatchResponse {
                command: "prompt".to_string(),
                summary: format!(
                    "Sent prompt to session {} (run {created_run_id})",
                    session.id
                ),
                data: json!({
                    "session_id": session.id.0,
                    "run_id": created_run_id,
                    "workflow_stage": workflow_stage,
                }),
            },
        ));
    }

    let mut conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let outcome = LocalStore::apply_command(&mut conn, &cmd, now).map_err(storage_error_to_http)?;
    let workflow_stage = match &outcome {
        CommandApplyOutcome::Applied { run, events, .. } => {
            state.publish_events(&session.id.0, events);
            state.notify_outbox();
            state.notify_dsh_subscriber();
            state.notify_scheduler();
            run.as_ref()
                .map_or(WorkflowStage::Implementation, |r| r.workflow_stage)
        }
        CommandApplyOutcome::AlreadyExists { run, .. } => run
            .as_ref()
            .map_or(WorkflowStage::Implementation, |r| r.workflow_stage),
    };
    Ok((
        StatusCode::CREATED,
        ChatDispatchResponse {
            command: "prompt".to_string(),
            summary: format!("Sent prompt to session {} (run {})", session.id, run_id),
            data: json!({
                "session_id": session.id.0,
                "run_id": run_id.0,
                "workflow_stage": workflow_stage,
            }),
        },
    ))
}
