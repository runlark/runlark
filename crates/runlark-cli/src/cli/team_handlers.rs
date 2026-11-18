use crate::cli::client::{DaemonHttpClient, print_output};
use crate::cli::run_chat_handlers::CliResult;
use crate::cli::{RunnerAction, TeamAction};
use serde_json::json;

/// Promotes a solo project to team ownership via `POST /api/v1/projects/{id}/promote-team`.
///
/// # Errors
/// Returns an error if active runs block promotion or the daemon/server request fails.
pub async fn handle_project_promote_team(
    client: &DaemonHttpClient,
    project_id: &str,
    server: String,
    workspace: String,
    token: Option<String>,
    json_mode: bool,
) -> CliResult {
    let mut body = json!({
        "server_url": server,
        "workspace_id": workspace,
    });
    if let Some(tok) = token {
        body["bearer_token"] = json!(tok);
    }
    let val = client
        .post_json(
            &format!("/api/v1/projects/{project_id}/promote-team"),
            Some(&body),
        )
        .await?;
    print_output(&val, json_mode, |v| {
        format!(
            "Promoted project {} -> workspace {} (epoch={}, cursor={})",
            v["ack"]["project_id"].as_str().unwrap_or(""),
            v["ack"]["workspace_id"].as_str().unwrap_or(""),
            v["ack"]["ownership_epoch"].as_u64().unwrap_or(1),
            v["ack"]["initial_cursor"].as_u64().unwrap_or(0)
        )
    })
}

/// Appends an immutable comment onto a task timeline.
///
/// # Errors
/// Returns an error if the daemon request fails.
pub async fn handle_task_comment(
    client: &DaemonHttpClient,
    task_id: &str,
    message: String,
    run_id: Option<String>,
    json_mode: bool,
) -> CliResult {
    let mut body = json!({ "body": message });
    if let Some(rid) = run_id {
        body["run_id"] = json!(rid);
    }
    let val = client
        .post_json(&format!("/api/v1/tasks/{task_id}/comments"), Some(&body))
        .await?;
    print_output(&val, json_mode, |v| {
        if v["delivery"].as_str() == Some("queued_offline") {
            format!(
                "Comment queued offline as draft {}",
                v["draft"]["id"].as_str().unwrap_or("")
            )
        } else {
            format!(
                "Comment {} added by {}",
                v["comment"]["id"].as_str().unwrap_or(""),
                v["comment"]["author_display_name"].as_str().unwrap_or("")
            )
        }
    })
}

/// Executes `runlark team` subcommands.
///
/// # Errors
/// Returns an error if the daemon request fails.
pub async fn handle_team(
    client: &DaemonHttpClient,
    action: TeamAction,
    json_mode: bool,
) -> CliResult {
    match action {
        TeamAction::Login {
            server,
            token,
            workspace,
        } => {
            let mut body = json!({
                "server_url": server,
                "bearer_token": token,
            });
            if let Some(wid) = workspace {
                body["workspace_id"] = json!(wid);
            }
            let val = client.post_json("/api/v1/team/login", Some(&body)).await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Logged in as {} ({}) @ {}",
                    v["display_name"].as_str().unwrap_or(""),
                    v["user_id"].as_str().unwrap_or(""),
                    v["server_url"].as_str().unwrap_or("")
                )
            })
        }
        TeamAction::Members { workspace } => {
            let val = client
                .get_json(&format!("/api/v1/team/workspaces/{workspace}/members"))
                .await?;
            print_output(&val, json_mode, |v| {
                v.as_array().map_or_else(String::new, |arr| {
                    arr.iter()
                        .map(|m| {
                            format!(
                                "{}  [{}]  {}",
                                m["user_id"].as_str().unwrap_or(""),
                                m["role"].as_str().unwrap_or(""),
                                m["display_name"].as_str().unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
            })
        }
        TeamAction::Invite {
            workspace,
            invitee,
            role,
            ttl_ms,
        } => handle_team_invite(client, workspace, invitee, role, ttl_ms, json_mode).await,
        TeamAction::AcceptInvite {
            server,
            token,
            name,
            user_id,
        } => handle_team_accept_invite(client, server, token, name, user_id, json_mode).await,
        TeamAction::Sync { workspace } => handle_team_sync(client, workspace, json_mode).await,
        TeamAction::Drafts { project_id } => {
            let params: Vec<(&str, String)> = project_id
                .into_iter()
                .map(|pid| ("project_id", pid))
                .collect();
            let val = client
                .get_json_query("/api/v1/team/drafts", &params)
                .await?;
            print_output(&val, json_mode, |v| {
                let drafts = v["drafts"].as_array().map_or(0, Vec::len);
                let outbox = v["outbox"].as_array().map_or(0, Vec::len);
                format!("Drafts: {drafts} | Sync outbox entries: {outbox}")
            })
        }
    }
}

/// Executes `runlark runner` subcommands.
///
/// # Errors
/// Returns an error if the daemon request fails.
pub async fn handle_runner(
    client: &DaemonHttpClient,
    action: RunnerAction,
    json_mode: bool,
) -> CliResult {
    match action {
        RunnerAction::Register {
            name,
            workspace,
            runner_id,
        } => {
            let mut body = json!({ "name": name });
            if let Some(wid) = workspace {
                body["workspace_id"] = json!(wid);
            }
            if let Some(rid) = runner_id {
                body["runner_id"] = json!(rid);
            }
            let val = client
                .post_json("/api/v1/team/runners/register", Some(&body))
                .await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Registered runner {} ({}) status={}",
                    v["id"].as_str().unwrap_or(""),
                    v["name"].as_str().unwrap_or(""),
                    v["status"].as_str().unwrap_or("")
                )
            })
        }
        RunnerAction::Status { workspace } => {
            let params: Vec<(&str, String)> =
                workspace.into_iter().map(|w| ("workspace_id", w)).collect();
            let val = client
                .get_json_query("/api/v1/team/runners/status", &params)
                .await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Runner {} ({}) buffered_events={} workspace_runners={}",
                    v["local_runner_id"].as_str().unwrap_or("unregistered"),
                    v["local_runner_name"].as_str().unwrap_or("-"),
                    v["buffered_events_count"].as_u64().unwrap_or(0),
                    v["workspace_runners"].as_array().map_or(0, Vec::len)
                )
            })
        }
        RunnerAction::Assign {
            run_id,
            runner_id,
            workspace,
            expected_revision,
        } => {
            let mut body = json!({ "runner_id": runner_id });
            if let Some(wid) = workspace {
                body["workspace_id"] = json!(wid);
            }
            if let Some(rev) = expected_revision {
                body["expected_revision"] = json!(rev);
            }
            let val = client
                .post_json(&format!("/api/v1/team/runs/{run_id}/assign"), Some(&body))
                .await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Assigned run {} -> runner {} (lease_epoch={})",
                    v["run"]["id"].as_str().unwrap_or(""),
                    v["ownership"]["assigned_runner_id"].as_str().unwrap_or(""),
                    v["ownership"]["lease_epoch"].as_u64().unwrap_or(0)
                )
            })
        }
        RunnerAction::CancelUnreachable {
            run_id,
            reason,
            workspace,
            expected_revision,
        } => {
            let mut body = json!({ "reason": reason });
            if let Some(wid) = workspace {
                body["workspace_id"] = json!(wid);
            }
            if let Some(rev) = expected_revision {
                body["expected_revision"] = json!(rev);
            }
            let val = client
                .post_json(
                    &format!("/api/v1/team/runs/{run_id}/cancel-unreachable"),
                    Some(&body),
                )
                .await?;
            print_output(&val, json_mode, |v| {
                format!(
                    "Cancelled unreachable run {} (cursor_seq={})",
                    v["aggregate_id"].as_str().unwrap_or(""),
                    v["cursor_seq"].as_u64().unwrap_or(0)
                )
            })
        }
    }
}

async fn handle_team_invite(
    client: &DaemonHttpClient,
    workspace: String,
    invitee: String,
    role: String,
    ttl_ms: Option<u64>,
    json_mode: bool,
) -> CliResult {
    let mut body = json!({
        "invitee_hint": invitee,
        "role": role,
    });
    if let Some(ttl) = ttl_ms {
        body["ttl_ms"] = json!(ttl);
    }
    let val = client
        .post_json(
            &format!("/api/v1/team/workspaces/{workspace}/invitations"),
            Some(&body),
        )
        .await?;
    print_output(&val, json_mode, |v| {
        format!(
            "Invite {} created (role={}) token={}",
            v["invitation"]["id"].as_str().unwrap_or(""),
            v["invitation"]["role"].as_str().unwrap_or(""),
            v["invite_token"].as_str().unwrap_or("")
        )
    })
}

async fn handle_team_accept_invite(
    client: &DaemonHttpClient,
    server: String,
    token: String,
    name: String,
    user_id: Option<String>,
    json_mode: bool,
) -> CliResult {
    let mut body = json!({
        "server_url": server,
        "invite_token": token,
        "display_name": name,
    });
    if let Some(uid) = user_id {
        body["user_id"] = json!(uid);
    }
    let val = client
        .post_json("/api/v1/team/invitations/accept", Some(&body))
        .await?;
    print_output(&val, json_mode, |v| {
        format!(
            "Joined workspace {} as {} ({})",
            v["member"]["workspace_id"].as_str().unwrap_or(""),
            v["member"]["display_name"].as_str().unwrap_or(""),
            v["member"]["role"].as_str().unwrap_or("")
        )
    })
}

async fn handle_team_sync(
    client: &DaemonHttpClient,
    workspace: Option<String>,
    json_mode: bool,
) -> CliResult {
    let body = workspace.map_or_else(|| json!({}), |w| json!({ "workspace_id": w }));
    let val = client.post_json("/api/v1/team/sync", Some(&body)).await?;
    print_output(&val, json_mode, |v| {
        format!(
            "Synced {} workspaces: snapshots={} events={} outbox_acked={} conflicts={} cas_up={} cas_down={}",
            v["workspaces_synced"].as_u64().unwrap_or(0),
            v["snapshots_applied"].as_u64().unwrap_or(0),
            v["events_applied"].as_u64().unwrap_or(0),
            v["outbox_acked"].as_u64().unwrap_or(0),
            v["outbox_conflicts"].as_u64().unwrap_or(0),
            v["cas_uploaded"].as_u64().unwrap_or(0),
            v["cas_downloaded"].as_u64().unwrap_or(0)
        )
    })
}
