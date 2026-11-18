use crate::routes::error::{ApiError, api_error, storage_error_to_http, team_server_error_to_http};
pub use crate::routes::team_decisions::{
    TeamAcceptParams, TeamRejectParams, execute_team_accept, execute_team_approve_plan,
    execute_team_reject, sync_run_evidence_to_server, validate_team_plan_approval_eligible,
};
use crate::runner_agent::{RunnerAgentWorker, RunnerStateInput};
use crate::state::AppState;
use crate::team_sync::{TeamCommandSubmissionResult, TeamSyncWorker};
use axum::http::StatusCode;
use runlark_domain::{
    ProjectId, ProjectOwnershipMode, Run, RunId, RunStatus, RunnerId, SessionCommand, Task, TaskId,
    TeamCommand, WorkspaceId,
};
use runlark_local::{SessionStore, SyncStore, TaskStore, TeamAuthStore};

/// Resolved server connection context for a `TeamManaged` project.
#[derive(Debug, Clone)]
pub struct TeamProjectContext {
    pub workspace_id: WorkspaceId,
    pub server_url: String,
    pub bearer_token: String,
    pub runner_id: Option<RunnerId>,
    pub display_name: String,
}

/// Resolved team ownership context for a [`SessionCommand`] target.
#[derive(Debug, Clone)]
pub struct SessionCommandTeamContext {
    pub run: Option<Run>,
    pub task: Option<Task>,
    pub context: TeamProjectContext,
}

/// Returns `Some(TeamProjectContext)` if `project_id` is `TeamManaged`, or `None` if `SoloLocal`.
///
/// # Errors
/// Returns `503 Service Unavailable` if the project is `TeamManaged` without an active login token.
pub fn resolve_project_team_context(
    state: &AppState,
    project_id: &ProjectId,
) -> Result<Option<TeamProjectContext>, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let mode = SyncStore::get_ownership_mode(&conn, project_id).map_err(storage_error_to_http)?;
    let ProjectOwnershipMode::TeamManaged {
        workspace_id,
        server_url,
        ..
    } = mode
    else {
        return Ok(None);
    };
    let auth = TeamAuthStore::get_by_server(&conn, &server_url)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "team_server_offline",
                format!("No team auth session for {server_url}; run `runlark team login` first"),
            )
        })?;
    Ok(Some(TeamProjectContext {
        workspace_id,
        server_url: auth.server_url,
        bearer_token: auth.bearer_token,
        runner_id: auth.runner_id,
        display_name: auth.display_name,
    }))
}

/// Loads `task_id` and resolves its project's [`TeamProjectContext`] if `TeamManaged`.
///
/// # Errors
/// Returns `404` if task missing, or `503` if `TeamManaged` without an auth session.
pub fn resolve_task_team_context(
    state: &AppState,
    task_id: &TaskId,
) -> Result<Option<(Task, TeamProjectContext)>, ApiError> {
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
    drop(conn);
    let Some(ctx) = resolve_project_team_context(state, &task.project_id)? else {
        return Ok(None);
    };
    Ok(Some((task, ctx)))
}

/// Resolves [`TeamProjectContext`] for `run` via its `project_id` or linked `task_id`.
///
/// # Errors
/// Returns `503` if `TeamManaged` without a login token, or `500` on storage failure.
pub fn resolve_run_team_context(
    state: &AppState,
    run: &Run,
) -> Result<Option<TeamProjectContext>, ApiError> {
    if let Some(ref pid) = run.project_id {
        return resolve_project_team_context(state, pid);
    }
    if let Some(ref tid) = run.task_id {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        if let Some(task) = TaskStore::get_task(&conn, tid).map_err(storage_error_to_http)? {
            drop(conn);
            return resolve_project_team_context(state, &task.project_id);
        }
    }
    Ok(None)
}

/// Resolves [`SessionCommandTeamContext`] if `command` targets a `TeamManaged` session, task, or run.
///
/// # Errors
/// Returns `503` if `TeamManaged` without a login token, or `500` on storage failure.
pub fn resolve_session_command_team_context(
    state: &AppState,
    command: &SessionCommand,
) -> Result<Option<SessionCommandTeamContext>, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let session =
        SessionStore::get_session(&conn, command.session_id()).map_err(storage_error_to_http)?;
    let run = match SessionStore::get_run(&conn, command.run_id()).map_err(storage_error_to_http)? {
        Some(r) => Some(r),
        None => match session.as_ref().and_then(|s| s.active_run_id.as_ref()) {
            Some(aid) => SessionStore::get_run(&conn, aid).map_err(storage_error_to_http)?,
            None => None,
        },
    };
    let task_id = run
        .as_ref()
        .and_then(|r| r.task_id.clone())
        .or_else(|| session.as_ref().map(|s| s.task_id.clone()));
    let task = match task_id {
        Some(ref tid) => TaskStore::get_task(&conn, tid).map_err(storage_error_to_http)?,
        None => None,
    };
    drop(conn);

    let project_id = run
        .as_ref()
        .and_then(|r| r.project_id.clone())
        .or_else(|| task.as_ref().map(|t| t.project_id.clone()));
    let Some(pid) = project_id else {
        return Ok(None);
    };
    let Some(context) = resolve_project_team_context(state, &pid)? else {
        return Ok(None);
    };
    Ok(Some(SessionCommandTeamContext { run, task, context }))
}

/// Submits an authoritative [`TeamCommand`] to the collaboration server and syncs the resulting
/// events into local `SQLite`. Never commits locally if the server is offline or rejects.
///
/// # Errors
/// Returns `503` if server unreachable, `409` on conflict, `403` on RBAC denial.
pub async fn submit_team_command_checked(
    state: &AppState,
    ctx: &TeamProjectContext,
    command: &TeamCommand,
) -> Result<TeamCommandSubmissionResult, ApiError> {
    let worker = TeamSyncWorker::new(state.clone());
    worker
        .submit_command_and_sync(
            &ctx.server_url,
            &ctx.bearer_token,
            &ctx.workspace_id,
            command,
        )
        .await
        .map_err(team_server_error_to_http)
}

/// Verifies the collaboration server is reachable and pulls the latest workspace state before
/// allowing a team-managed side effect such as PR publication.
///
/// # Errors
/// Returns `503` if server is unreachable.
pub async fn verify_server_reachable_and_synced(
    state: &AppState,
    ctx: &TeamProjectContext,
) -> Result<(), ApiError> {
    let worker = TeamSyncWorker::new(state.clone());
    worker
        .sync_workspace(&ctx.server_url, &ctx.bearer_token, &ctx.workspace_id)
        .await
        .map_err(team_server_error_to_http)?;
    Ok(())
}

/// Reports (or offline-buffers) `RunStatus::Running` to `runlark-server` when a `TeamManaged`
/// session resume or question answer transitions a run that this daemon actively leases.
///
/// Local DSH session commands (`ResumeSession`, `AnswerQuestion`) do not have separate
/// [`TeamCommand`] variants because the assigned runner owns local DSH host execution and
/// propagates resulting run-state transitions back to the server via fenced runner reports.
pub async fn sync_team_run_resumed_if_leased(
    state: &AppState,
    ctx: &TeamProjectContext,
    run_id: &RunId,
    event_type: &str,
    payload_json: &str,
) {
    let Some(ref runner_id) = ctx.runner_id else {
        return;
    };
    let ownership = {
        let Ok(conn) = state.get_connection() else {
            return;
        };
        SyncStore::get_run_ownership(&conn, run_id)
            .ok()
            .flatten()
            .unwrap_or_default()
    };
    if ownership.runner_unreachable
        || ownership.lease_epoch.0 == 0
        || ownership.assigned_runner_id.as_ref() != Some(runner_id)
    {
        return;
    }
    if let Err(err) = RunnerAgentWorker::new(state.clone())
        .report_or_buffer_run_state(RunnerStateInput {
            workspace_id: &ctx.workspace_id,
            run_id,
            runner_id,
            lease_epoch: ownership.lease_epoch,
            status: Some(RunStatus::Running),
            workflow_stage: None,
            head_sha: None,
            base_sha: None,
            event_type,
            payload_json,
        })
        .await
    {
        tracing::warn!(
            run_id = %run_id,
            "Failed to report resumed team run state after local DSH command: {err}"
        );
    }
}
