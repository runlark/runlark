use crate::cli::client::{DaemonHttpClient, print_output};
use crate::cli::{PluginAction, QuestionAction, RunAction};
use crate::plugin;
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

pub type CliResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

pub(super) async fn handle_task_pr(
    client: &DaemonHttpClient,
    id: &str,
    title: Option<String>,
    base_branch: Option<String>,
    draft: bool,
    json_mode: bool,
) -> CliResult {
    let mut body = json!({ "draft": draft });
    if let Some(t) = title {
        body["title"] = json!(t);
    }
    if let Some(b) = base_branch {
        body["base_branch"] = json!(b);
    }
    let val = client
        .post_json(&format!("/api/v1/tasks/{id}/pr"), Some(&body))
        .await?;
    print_output(&val, json_mode, |v| {
        format!(
            "PR {} -> {}",
            v["outcome"].as_str().unwrap_or(""),
            v["pr_url"]
                .as_str()
                .unwrap_or_else(|| v["reason"].as_str().unwrap_or(""))
        )
    })
}

pub(super) async fn handle_task_board(
    client: &DaemonHttpClient,
    project_id: Option<String>,
    json_mode: bool,
) -> CliResult {
    let params: Vec<(&str, String)> = project_id
        .into_iter()
        .map(|pid| ("project_id", pid))
        .collect();
    let val = client.get_json_query("/api/v1/board", &params).await?;
    print_output(&val, json_mode, |v| {
        format!(
            "Board: {} tasks | slots {}/{} | queued={} | known_cost={} unknown_attempts={}",
            v["tasks"].as_array().map_or(0, Vec::len),
            v["slots"]["used"].as_u64().unwrap_or(0),
            v["slots"]["max"].as_u64().unwrap_or(2),
            v["queued_runs"].as_array().map_or(0, Vec::len),
            v["cost_summary"]["known_micro_usd"].as_u64().unwrap_or(0),
            v["cost_summary"]["unknown_attempts"].as_u64().unwrap_or(0)
        )
    })
}

/// Executes `runlark run` subcommands.
///
/// # Errors
/// Returns an error if the daemon request fails.
pub async fn handle_run(
    client: &DaemonHttpClient,
    action: RunAction,
    json_mode: bool,
) -> CliResult {
    match action {
        RunAction::List { task_id, status } => {
            list_runs_cmd(client, task_id, status, json_mode).await
        }
        RunAction::Show { id } => {
            let val = client.get_json(&format!("/api/v1/runs/{id}")).await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Run {} status={} worktree={}",
                    v["run_id"].as_str().unwrap_or(""),
                    v["status"].as_str().unwrap_or(""),
                    v["worktree_path"].as_str().unwrap_or("")
                )
            })
        }
        RunAction::Events { id, after_seq } => {
            let params: Vec<(&str, String)> = after_seq
                .into_iter()
                .map(|s| ("after_seq", s.to_string()))
                .collect();
            let val = client
                .get_json_query(&format!("/api/v1/runs/{id}/events"), &params)
                .await?;
            print_output(&val, json_mode, format_run_events)
        }
        RunAction::History { id } => {
            let val = client
                .get_json(&format!("/api/v1/runs/{id}/history"))
                .await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Run {} attempts={} decisions={} artifacts={} events={}",
                    v["run"]["id"].as_str().unwrap_or(""),
                    v["attempts"].as_array().map_or(0, Vec::len),
                    v["decisions"].as_array().map_or(0, Vec::len),
                    v["artifacts"].as_array().map_or(0, Vec::len),
                    v["events"].as_array().map_or(0, Vec::len)
                )
            })
        }
        RunAction::Diff { id } => {
            let val = client.get_json(&format!("/api/v1/runs/{id}/diff")).await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Diff {} ({}..{}, {} bytes)\n{}",
                    v["digest_sha256"].as_str().unwrap_or(""),
                    v["base_sha"].as_str().unwrap_or(""),
                    v["head_sha"].as_str().unwrap_or(""),
                    v["diff_bytes"].as_u64().unwrap_or(0),
                    v["diff"].as_str().unwrap_or("")
                )
            })
        }
        RunAction::Verify {
            id,
            task_id,
            package_name,
        } => {
            let body = json!({ "task_id": task_id, "package_name": package_name });
            let val = client
                .post_json(&format!("/api/v1/runs/{id}/verify"), Some(&body))
                .await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Verdict={} ({}, fuel={}/{})",
                    v["verdict"].as_str().unwrap_or(""),
                    v["summary"].as_str().unwrap_or(""),
                    v["fuel_consumed"].as_u64().unwrap_or(0),
                    v["fuel_limit"].as_u64().unwrap_or(0)
                )
            })
        }
    }
}

async fn list_runs_cmd(
    client: &DaemonHttpClient,
    task_id: Option<String>,
    status: Option<String>,
    json_mode: bool,
) -> CliResult {
    let mut params = Vec::new();
    if let Some(tid) = task_id {
        params.push(("task_id", tid));
    }
    if let Some(st) = status {
        params.push(("status", st));
    }
    let val = client.get_json_query("/api/v1/runs", &params).await?;
    print_output(&val, json_mode, |v| {
        v.as_array().map_or_else(String::new, |arr| {
            if arr.is_empty() {
                return "No runs found.".to_string();
            }
            arr.iter()
                .map(|r| {
                    format!(
                        "{}  {:?}  stage={}  wt={}",
                        r["id"].as_str().unwrap_or(""),
                        r["status"].as_str().unwrap_or(""),
                        r["workflow_stage"].as_str().unwrap_or(""),
                        r["worktree_path"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
    })
}

fn format_run_events(v: &Value) -> String {
    v.as_array().map_or_else(String::new, |arr| {
        arr.iter()
            .map(|ev| {
                format!(
                    "#{}  {}",
                    ev["seq"].as_u64().unwrap_or(0),
                    ev["event_type"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    })
}

/// Executes `runlark question` subcommands.
///
/// # Errors
/// Returns an error if the daemon request fails.
pub async fn handle_question(
    client: &DaemonHttpClient,
    action: QuestionAction,
    json_mode: bool,
) -> CliResult {
    match action {
        QuestionAction::List { session_id, run_id } => {
            let mut params = Vec::new();
            if let Some(sid) = session_id {
                params.push(("session_id", sid));
            }
            if let Some(rid) = run_id {
                params.push(("run_id", rid));
            }
            let val = client.get_json_query("/api/v1/questions", &params).await?;
            print_output(&val, json_mode, |v| {
                v.as_array().map_or_else(String::new, |arr| {
                    if arr.is_empty() {
                        return "No pending questions.".to_string();
                    }
                    arr.iter()
                        .map(|q| {
                            format!(
                                "{} (run={}): {}",
                                q["question_id"].as_str().unwrap_or(""),
                                q["run_id"].as_str().unwrap_or(""),
                                q["prompt"].as_str().unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
            })
        }
        QuestionAction::Answer { id, answer } => {
            let body = json!({ "answer": answer });
            let val = client
                .post_json(&format!("/api/v1/questions/{id}/answer"), Some(&body))
                .await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Answered {} (run_status={})",
                    v["question_id"].as_str().unwrap_or(""),
                    v["run_status"].as_str().unwrap_or("")
                )
            })
        }
    }
}

/// Sends a single slash command or prompt via `runlark chat send`.
///
/// # Errors
/// Returns an error if the daemon request fails.
pub async fn handle_chat_send(
    client: &DaemonHttpClient,
    input: String,
    project_id: Option<String>,
    session_id: Option<String>,
    run_id: Option<String>,
    json_mode: bool,
) -> CliResult {
    let mut body = json!({ "input": input });
    if let Some(pid) = project_id {
        body["project_id"] = json!(pid);
    }
    if let Some(sid) = session_id {
        body["session_id"] = json!(sid);
    }
    if let Some(rid) = run_id {
        body["run_id"] = json!(rid);
    }
    let val = client
        .post_json("/api/v1/chat/dispatch", Some(&body))
        .await?;
    print_output(&val, json_mode, |v| {
        v["summary"].as_str().unwrap_or("").to_string()
    })
}

/// Starts an interactive chat REPL against the local daemon.
///
/// # Errors
/// Returns an error if reading stdin fails.
pub async fn handle_chat_repl(
    client: &DaemonHttpClient,
    project_id: Option<String>,
    session_id: Option<String>,
) -> CliResult {
    println!("Runlark chat REPL (type /help, or `exit` / Ctrl-D to quit)");
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line_res in stdin.lock().lines() {
        let line = line_res?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if matches!(trimmed, "exit" | "quit" | "/exit" | "/quit") {
            break;
        }
        let mut body = json!({ "input": trimmed });
        if let Some(ref pid) = project_id {
            body["project_id"] = json!(pid);
        }
        if let Some(ref sid) = session_id {
            body["session_id"] = json!(sid);
        }
        match client.post_json("/api/v1/chat/dispatch", Some(&body)).await {
            Ok(val) => println!("{}", val["summary"].as_str().unwrap_or("ok")),
            Err(err) => eprintln!("Error: {err}"),
        }
        print!("> ");
        let _ = stdout.flush();
    }
    Ok(())
}

/// Executes `runlark plugin` subcommands.
///
/// # Errors
/// Returns an error if validation, inspection, or daemon HTTP call fails.
pub async fn handle_plugin(
    client: &DaemonHttpClient,
    action: PluginAction,
    json_mode: bool,
) -> CliResult {
    match action {
        PluginAction::Validate { path, json } => {
            plugin::run_plugin_validate(&path, json || json_mode)
        }
        PluginAction::Inspect { path, json } => {
            plugin::run_plugin_inspect(&path, json || json_mode)
        }
        PluginAction::Install { path } => {
            let body = json!({ "path": path.display().to_string() });
            let val = client.post_json("/api/v1/plugins", Some(&body)).await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Installed {}@{} (digest: {})",
                    v["name"].as_str().unwrap_or(""),
                    v["version"].as_str().unwrap_or(""),
                    v["package_digest"].as_str().unwrap_or("")
                )
            })
        }
        PluginAction::List => {
            let val = client.get_json("/api/v1/plugins").await?;
            print_output(&val, json_mode, |v| {
                v.as_array().map_or_else(String::new, |arr| {
                    if arr.is_empty() {
                        return "No plugins installed.".to_string();
                    }
                    arr.iter()
                        .map(|p| {
                            format!(
                                "{}@{}  {}",
                                p["name"].as_str().unwrap_or(""),
                                p["version"].as_str().unwrap_or(""),
                                p["package_digest"].as_str().unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
            })
        }
    }
}
