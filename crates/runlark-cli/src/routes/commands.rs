use crate::routes::error::{ApiError, api_error, run_status_str, storage_error_to_http};
use crate::routes::team_gate::{self, SessionCommandTeamContext};
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use runlark_domain::{
    CancelTeamRunCommand, CreateTeamRunCommand, RunId, SessionCommand, TeamCommand, Timestamp,
};
use runlark_local::{CommandApplyOutcome, LocalStore, SessionStore};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct SubmitCommandResponse {
    pub command_id: String,
    pub status: String,
    pub current_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_status: Option<String>,
}

/// Submits, persists, and atomically applies a [`SessionCommand`] before notifying the outbox.
///
/// For `TeamManaged` sessions/tasks/runs, enforces server reachability and routes run creation /
/// cancellation through authoritative [`TeamCommand`] submission without mutating `SQLite` offline.
///
/// # Errors
/// Returns:
/// - `409 Conflict` on `RevisionMismatch`, `CommandPayloadConflict`, or state conflict transitions
/// - `404 Not Found` if the target session, run, or question does not exist
/// - `422 Unprocessable Entity` on invalid state/session mismatch
/// - `503 Service Unavailable` if a `TeamManaged` target's collaboration server is offline
/// - `500 Internal Server Error` if opening or writing `SQLite` fails
pub async fn submit_command(
    State(state): State<AppState>,
    Json(command): Json<SessionCommand>,
) -> Result<(StatusCode, Json<SubmitCommandResponse>), ApiError> {
    let now = Timestamp(AppState::current_timestamp_ms());
    let team_ctx = team_gate::resolve_session_command_team_context(&state, &command)?;
    if let Some(team) = team_ctx.clone()
        && let Some(resp) = submit_team_session_command(&state, &command, team, now).await?
    {
        return Ok(resp);
    }
    let resp = apply_local_session_command(&state, &command, now)?;
    if resp.0 == StatusCode::CREATED
        && let Some(team) = team_ctx
        && matches!(
            command,
            SessionCommand::ResumeSession(_) | SessionCommand::AnswerQuestion(_)
        )
    {
        team_gate::sync_team_run_resumed_if_leased(
            &state,
            &team.context,
            command.run_id(),
            "session_resumed",
            &serde_json::json!({ "command_id": command.command_id().0 }).to_string(),
        )
        .await;
    }
    Ok(resp)
}

pub(crate) async fn submit_team_session_command(
    state: &AppState,
    command: &SessionCommand,
    team: SessionCommandTeamContext,
    now: Timestamp,
) -> Result<Option<(StatusCode, Json<SubmitCommandResponse>)>, ApiError> {
    match command {
        SessionCommand::StopRun(stop_cmd) => {
            let target_run = if let Some(r) = team.run {
                r
            } else {
                team_gate::verify_server_reachable_and_synced(state, &team.context).await?;
                load_run_or_404(state, &stop_cmd.run_id)?
            };
            let session_rev = load_session_revision(state, &stop_cmd.session_id)?;
            let expected_revision = if session_rev == Some(stop_cmd.expected_revision)
                || target_run.revision == stop_cmd.expected_revision
            {
                target_run.revision
            } else {
                stop_cmd.expected_revision
            };
            let reason = stop_cmd
                .reason
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "Cancelled by operator".to_string());
            let team_cmd = TeamCommand::CancelRun(CancelTeamRunCommand {
                command_id: stop_cmd.command_id.clone(),
                run_id: target_run.id.clone(),
                reason,
                expected_revision,
                created_at: now,
            });
            let res =
                team_gate::submit_team_command_checked(state, &team.context, &team_cmd).await?;
            Ok(Some(build_team_command_response(
                state,
                &stop_cmd.command_id,
                &RunId::from(res.aggregate_id),
                res.revision,
                res.idempotent_replay,
            )?))
        }
        SessionCommand::SendPrompt(send_cmd)
            if team
                .run
                .as_ref()
                .is_none_or(|r| r.status.is_terminal() && r.id != send_cmd.run_id) =>
        {
            let Some(task) = team.task else {
                team_gate::verify_server_reachable_and_synced(state, &team.context).await?;
                return Err(api_error(
                    StatusCode::CONFLICT,
                    "task_required_for_team_run",
                    "Team-managed runs must target a board task",
                ));
            };
            let session_rev = load_session_revision(state, &send_cmd.session_id)?;
            let expected_revision = if session_rev == Some(send_cmd.expected_revision)
                || task.revision == send_cmd.expected_revision
            {
                task.revision
            } else {
                send_cmd.expected_revision
            };
            let team_cmd = TeamCommand::CreateTeamRun(CreateTeamRunCommand {
                command_id: send_cmd.command_id.clone(),
                run_id: send_cmd.run_id.clone(),
                project_id: task.project_id,
                task_id: task.id,
                runner_id: team.context.runner_id.clone(),
                expected_revision,
                created_at: now,
            });
            let res =
                team_gate::submit_team_command_checked(state, &team.context, &team_cmd).await?;
            state.notify_scheduler();
            state.notify_runner();
            Ok(Some(build_team_command_response(
                state,
                &send_cmd.command_id,
                &RunId::from(res.aggregate_id),
                res.revision,
                res.idempotent_replay,
            )?))
        }
        SessionCommand::SendPrompt(_)
        | SessionCommand::ResumeSession(_)
        | SessionCommand::ChangeCwd(_)
        | SessionCommand::ArchiveSession(_)
        | SessionCommand::AnswerQuestion(_) => {
            team_gate::verify_server_reachable_and_synced(state, &team.context).await?;
            Ok(None)
        }
    }
}

fn load_run_or_404(state: &AppState, run_id: &RunId) -> Result<runlark_domain::Run, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    SessionStore::get_run(&conn, run_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "run_not_found",
                format!("Run not found: {run_id}"),
            )
        })
}

fn load_session_revision(
    state: &AppState,
    session_id: &runlark_domain::SessionId,
) -> Result<Option<runlark_domain::Revision>, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    Ok(SessionStore::get_session(&conn, session_id)
        .map_err(storage_error_to_http)?
        .map(|s| s.current_revision))
}

fn build_team_command_response(
    state: &AppState,
    command_id: &runlark_domain::CommandId,
    run_id: &RunId,
    revision: u64,
    idempotent_replay: bool,
) -> Result<(StatusCode, Json<SubmitCommandResponse>), ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let run = SessionStore::get_run(&conn, run_id).map_err(storage_error_to_http)?;
    let status_code = if idempotent_replay {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((
        status_code,
        Json(SubmitCommandResponse {
            command_id: command_id.to_string(),
            status: if idempotent_replay {
                "acked".to_string()
            } else {
                "dispatched".to_string()
            },
            current_revision: revision,
            run_id: Some(run_id.0.clone()),
            run_status: run.as_ref().map(|r| run_status_str(r.status).to_string()),
        }),
    ))
}

fn apply_local_session_command(
    state: &AppState,
    command: &SessionCommand,
    now: Timestamp,
) -> Result<(StatusCode, Json<SubmitCommandResponse>), ApiError> {
    let mut conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;

    let command_id = command.command_id().to_string();
    let outcome =
        LocalStore::apply_command(&mut conn, command, now).map_err(storage_error_to_http)?;

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
            state.notify_scheduler();
            Ok((
                StatusCode::CREATED,
                Json(SubmitCommandResponse {
                    command_id,
                    status,
                    current_revision: session.current_revision.0,
                    run_id: run.as_ref().map(|r| r.id.0.clone()),
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
            Json(SubmitCommandResponse {
                command_id,
                status,
                current_revision: session.current_revision.0,
                run_id: run.as_ref().map(|r| r.id.0.clone()),
                run_status: run.as_ref().map(|r| run_status_str(r.status).to_string()),
            }),
        )),
    }
}
