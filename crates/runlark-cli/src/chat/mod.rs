mod run_ops;
mod status_ops;
mod task_ops;

use crate::routes::error::{ApiError, api_error};
use crate::state::AppState;
use axum::http::StatusCode;
use runlark_domain::{ChatParseError, parse_chat_command};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
pub struct ChatDispatchRequest {
    pub input: String,
    pub project_id: Option<String>,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub actor: Option<String>,
    pub command_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatDispatchResponse {
    pub command: String,
    pub summary: String,
    pub data: Value,
}

const HELP_TEXT: &str = concat!(
    "Commands: /task [--quick] <title>, /task move <task_id> <stage>, /fix <title>, ",
    "/plan <task_id> <text>, /plan approve <task_id> [comment], /run <task_id> [--prompt ...], ",
    "/stop <run_id> [reason], /resume <run_id>, /answer <question_id> <answer>, ",
    "/accept <target_id> [comment], /reject <target_id> <reason>, ",
    "/pr <run_id> [--title ...] [--draft], /status [target_id], /board [project_id], ",
    "/budget [target_id], /help"
);

/// Executes a parsed chat or slash command against the daemon's unified domain/storage layer.
///
/// # Errors
/// Returns [`ApiError`] on parse errors (`400`), missing entities (`404`), conflicts (`409`),
/// or internal storage/execution failures (`500`/`502`).
pub async fn dispatch_chat_command(
    state: AppState,
    req: ChatDispatchRequest,
) -> Result<(StatusCode, ChatDispatchResponse), ApiError> {
    let parsed = parse_chat_command(&req.input).map_err(|e| chat_parse_error_to_api(&e))?;
    match parsed {
        runlark_domain::ParsedChatCommand::Help => Ok((
            StatusCode::OK,
            ChatDispatchResponse {
                command: "help".to_string(),
                summary: HELP_TEXT.to_string(),
                data: json!({ "help": HELP_TEXT }),
            },
        )),
        runlark_domain::ParsedChatCommand::CreateTask { title, kind } => {
            task_ops::exec_create_task(state, req, title, kind).await
        }
        runlark_domain::ParsedChatCommand::MoveTask { task_id, stage } => {
            task_ops::exec_move_task(state, req, &task_id, stage).await
        }
        runlark_domain::ParsedChatCommand::UpdatePlan { task_id, plan_text } => {
            task_ops::exec_update_plan(state, req, &task_id, plan_text).await
        }
        runlark_domain::ParsedChatCommand::ApprovePlan { task_id, comment } => {
            task_ops::exec_approve_plan(&state, &task_id, comment).await
        }
        runlark_domain::ParsedChatCommand::StartRun { task_id, prompt } => {
            task_ops::exec_start_run(state, req, &task_id, prompt).await
        }
        runlark_domain::ParsedChatCommand::StopRun { run_id, reason } => {
            run_ops::exec_stop_run(&state, &run_id, reason, req.command_id).await
        }
        runlark_domain::ParsedChatCommand::ResumeRun { run_id } => {
            run_ops::exec_resume_run(&state, &run_id, req.command_id).await
        }
        runlark_domain::ParsedChatCommand::AnswerQuestion {
            question_id,
            answer,
        } => run_ops::exec_answer_question(&state, &question_id, &answer, req.command_id).await,
        runlark_domain::ParsedChatCommand::Accept { target_id, comment } => {
            task_ops::exec_accept_target(state, req, &target_id, comment).await
        }
        runlark_domain::ParsedChatCommand::Reject { target_id, reason } => {
            task_ops::exec_reject_target(state, req, &target_id, reason).await
        }
        runlark_domain::ParsedChatCommand::CreatePr {
            run_id,
            title,
            draft,
        } => run_ops::exec_create_pr(&state, &run_id, title, draft, req.command_id).await,
        runlark_domain::ParsedChatCommand::Status { target_id } => {
            status_ops::exec_status(&state, target_id)
        }
        runlark_domain::ParsedChatCommand::Board { project_id } => {
            task_ops::exec_board(state, project_id.map(|p| p.0).or(req.project_id)).await
        }
        runlark_domain::ParsedChatCommand::Budget { target_id } => {
            status_ops::exec_budget(&state, target_id.or(req.run_id).or(req.project_id))
        }
        runlark_domain::ParsedChatCommand::Prompt { text } => {
            status_ops::exec_prompt(&state, &req, text).await
        }
    }
}

fn chat_parse_error_to_api(err: &ChatParseError) -> ApiError {
    api_error(
        StatusCode::BAD_REQUEST,
        "invalid_chat_command",
        err.to_string(),
    )
}
