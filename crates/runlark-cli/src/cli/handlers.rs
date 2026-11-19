use crate::cli::client::{DaemonHttpClient, print_output, validate_path_segment};
use crate::cli::run_chat_handlers::{handle_task_board, handle_task_pr};
use crate::cli::team_handlers::{TokenInput, handle_project_promote_team, handle_task_comment};
use crate::cli::{ProjectAction, TaskAction};
use serde_json::{Value, json};

pub use crate::cli::run_chat_handlers::{
    CliResult, handle_chat_repl, handle_chat_send, handle_plugin, handle_question, handle_run,
};

/// Executes `runlark project` subcommands.
///
/// # Errors
/// Returns an error if the daemon request fails.
pub async fn handle_project(
    client: &DaemonHttpClient,
    action: ProjectAction,
    json_mode: bool,
) -> CliResult {
    match action {
        ProjectAction::Create {
            id,
            name,
            repo_root,
            default_branch,
            max_attempts,
            max_duration_ms,
            max_spend_micro_usd,
        } => {
            let resolved_root = match repo_root {
                Some(root) => root,
                None => std::env::current_dir()?.display().to_string(),
            };
            let mut body = json!({
                "name": name,
                "repo_root": resolved_root,
                "default_branch": default_branch,
                "budget_policy": {
                    "max_attempts": max_attempts,
                    "max_duration_ms": max_duration_ms,
                    "max_spend_micro_usd": max_spend_micro_usd,
                    "allow_unknown_cost_repair": false
                }
            });
            if let Some(pid) = id {
                body["project_id"] = json!(pid);
            }
            let val = client.post_json("/api/v1/projects", Some(&body)).await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Project {} ({}) -> {}",
                    v["project_id"].as_str().unwrap_or(""),
                    v["name"].as_str().unwrap_or(""),
                    v["repo_root"].as_str().unwrap_or("")
                )
            })
        }
        ProjectAction::List => {
            let val = client.get_json("/api/v1/projects").await?;
            print_output(&val, json_mode, format_project_list)
        }
        ProjectAction::Show { id } => {
            let id = validate_path_segment(&id, "project_id")?;
            let val = client.get_json(&format!("/api/v1/projects/{id}")).await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "{} ({}) branch={} repo={}",
                    v["project_id"].as_str().unwrap_or(""),
                    v["name"].as_str().unwrap_or(""),
                    v["default_branch"].as_str().unwrap_or(""),
                    v["repo_root"].as_str().unwrap_or("")
                )
            })
        }
        ProjectAction::PromoteTeam {
            project_id,
            server,
            workspace,
            token,
            token_file,
            token_stdin,
        } => {
            handle_project_promote_team(
                client,
                &project_id,
                server,
                workspace,
                TokenInput {
                    token,
                    token_file,
                    token_stdin,
                },
                json_mode,
            )
            .await
        }
    }
}

fn format_project_list(v: &Value) -> String {
    v.as_array().map_or_else(String::new, |arr| {
        if arr.is_empty() {
            return "No projects registered.".to_string();
        }
        arr.iter()
            .map(|p| {
                format!(
                    "{}  {}  ({})",
                    p["project_id"].as_str().unwrap_or(""),
                    p["name"].as_str().unwrap_or(""),
                    p["repo_root"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    })
}

/// Executes `runlark task` subcommands.
///
/// # Errors
/// Returns an error if the daemon request fails.
pub async fn handle_task(
    client: &DaemonHttpClient,
    action: TaskAction,
    json_mode: bool,
) -> CliResult {
    match action {
        TaskAction::Create {
            id,
            project_id,
            title,
            description,
            quick,
        } => {
            let kind = if quick { "quick_fix" } else { "standard" };
            create_task_cmd(client, id, project_id, title, description, kind, json_mode).await
        }
        TaskAction::Fix {
            id,
            project_id,
            title,
            description,
        } => {
            create_task_cmd(
                client,
                id,
                project_id,
                title,
                description,
                "quick_fix",
                json_mode,
            )
            .await
        }
        TaskAction::List { project_id, stage } => {
            list_tasks_cmd(client, project_id, stage, json_mode).await
        }
        TaskAction::Show { id } => {
            let id = validate_path_segment(&id, "task_id")?;
            let val = client.get_json(&format!("/api/v1/tasks/{id}")).await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Task {} [{}] workflow_stage={} known_cost={} unknown_attempts={}",
                    v["task"]["id"].as_str().unwrap_or(""),
                    v["task"]["stage"].as_str().unwrap_or(""),
                    v["task"]["workflow_stage"].as_str().unwrap_or(""),
                    v["cost_summary"]["known_micro_usd"].as_u64().unwrap_or(0),
                    v["cost_summary"]["unknown_attempts"].as_u64().unwrap_or(0)
                )
            })
        }
        TaskAction::Move { id, stage, actor } => {
            let id = validate_path_segment(&id, "task_id")?;
            let body = json!({ "stage": stage, "actor_name": actor });
            let val = client
                .post_json(&format!("/api/v1/tasks/{id}/move"), Some(&body))
                .await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Moved {} -> {}",
                    v["id"].as_str().unwrap_or(""),
                    v["stage"].as_str().unwrap_or("")
                )
            })
        }
        TaskAction::Clarify {
            id,
            clarification,
            no_advance,
        } => {
            let id = validate_path_segment(&id, "task_id")?;
            let body = json!({ "clarification": clarification, "advance_to_plan": !no_advance });
            let val = client
                .post_json(&format!("/api/v1/tasks/{id}/clarify"), Some(&body))
                .await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Clarified {} (workflow_stage={})",
                    v["task"]["id"].as_str().unwrap_or(""),
                    v["workflow_stage"].as_str().unwrap_or("")
                )
            })
        }
        TaskAction::Plan {
            id,
            plan_text,
            approve,
        } => {
            let id = validate_path_segment(&id, "task_id")?;
            let body = json!({ "plan_text": plan_text, "approve": approve });
            let val = client
                .post_json(&format!("/api/v1/tasks/{id}/plan"), Some(&body))
                .await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Updated plan for {} (v{}, workflow_stage={})",
                    v["id"].as_str().unwrap_or(""),
                    v["plan_version"].as_u64().unwrap_or(1),
                    v["workflow_stage"].as_str().unwrap_or("")
                )
            })
        }
        other => handle_task_lifecycle(client, other, json_mode).await,
    }
}

async fn create_task_cmd(
    client: &DaemonHttpClient,
    id: Option<String>,
    project_id: Option<String>,
    title: String,
    description: String,
    kind: &str,
    json_mode: bool,
) -> CliResult {
    let mut body = json!({
        "title": title,
        "description": description,
        "kind": kind,
    });
    if let Some(tid) = id {
        body["task_id"] = json!(tid);
    }
    if let Some(pid) = project_id {
        body["project_id"] = json!(pid);
    }
    let val = client.post_json("/api/v1/tasks", Some(&body)).await?;
    print_output(&val, json_mode, |v| {
        format!(
            "Task {} [{}] ({}): {}",
            v["id"].as_str().unwrap_or(""),
            v["stage"].as_str().unwrap_or(""),
            v["kind"].as_str().unwrap_or(""),
            v["title"].as_str().unwrap_or("")
        )
    })
}

async fn list_tasks_cmd(
    client: &DaemonHttpClient,
    project_id: Option<String>,
    stage: Option<String>,
    json_mode: bool,
) -> CliResult {
    let mut params = Vec::new();
    if let Some(pid) = project_id {
        params.push(("project_id", pid));
    }
    if let Some(st) = stage {
        params.push(("stage", st));
    }
    let val = client.get_json_query("/api/v1/tasks", &params).await?;
    print_output(&val, json_mode, |v| {
        v.as_array().map_or_else(String::new, |arr| {
            if arr.is_empty() {
                return "No tasks found.".to_string();
            }
            arr.iter()
                .map(|t| {
                    format!(
                        "{}  [{}] ({})  {}",
                        t["id"].as_str().unwrap_or(""),
                        t["stage"].as_str().unwrap_or(""),
                        t["workflow_stage"].as_str().unwrap_or(""),
                        t["title"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
    })
}

async fn handle_task_lifecycle(
    client: &DaemonHttpClient,
    action: TaskAction,
    json_mode: bool,
) -> CliResult {
    match action {
        TaskAction::Start { id, prompt } => {
            let id = validate_path_segment(&id, "task_id")?;
            let mut body = json!({});
            if let Some(p) = prompt {
                body["prompt"] = json!(p);
            }
            let val = client
                .post_json(&format!("/api/v1/tasks/{id}/start"), Some(&body))
                .await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Task {} -> run {} ({})",
                    v["task"]["id"].as_str().unwrap_or(""),
                    v["run"]["id"].as_str().unwrap_or(""),
                    v["run_status"].as_str().unwrap_or("")
                )
            })
        }
        TaskAction::Stop { run_id, reason } => {
            let cmd_text = reason.map_or_else(
                || format!("/stop {run_id}"),
                |r| format!("/stop {run_id} {r}"),
            );
            let body = json!({ "input": cmd_text });
            let val = client
                .post_json("/api/v1/chat/dispatch", Some(&body))
                .await?;
            print_output(&val, json_mode, |v| {
                v["summary"].as_str().unwrap_or("").to_string()
            })
        }
        TaskAction::Accept { id, actor, comment } => {
            let id = validate_path_segment(&id, "task_id")?;
            let mut body = json!({ "actor": actor });
            if let Some(c) = comment {
                body["comment"] = json!(c);
            }
            let val = client
                .post_json(&format!("/api/v1/tasks/{id}/accept"), Some(&body))
                .await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Accepted {} (decision={}, stage={})",
                    v["task"]["id"].as_str().unwrap_or(""),
                    v["decision_id"].as_str().unwrap_or(""),
                    v["task"]["stage"].as_str().unwrap_or("")
                )
            })
        }
        TaskAction::Reject { id, actor, reason } => {
            let id = validate_path_segment(&id, "task_id")?;
            let body = json!({ "actor": actor, "reason": reason });
            let val = client
                .post_json(&format!("/api/v1/tasks/{id}/reject"), Some(&body))
                .await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Rejected {} -> back to {} (decision={})",
                    v["task"]["id"].as_str().unwrap_or(""),
                    v["task"]["stage"].as_str().unwrap_or(""),
                    v["decision_id"].as_str().unwrap_or("")
                )
            })
        }
        TaskAction::Pr {
            id,
            title,
            base_branch,
            draft,
        } => handle_task_pr(client, &id, title, base_branch, draft, json_mode).await,
        TaskAction::Board { project_id } => handle_task_board(client, project_id, json_mode).await,
        TaskAction::Comment {
            task_id,
            message,
            run_id,
        } => handle_task_comment(client, &task_id, message, run_id, json_mode).await,
        _ => unreachable!(),
    }
}
