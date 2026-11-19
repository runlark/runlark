use crate::git_pr::{
    CreatePrOutcome, CreatePrRequest, GitPrError, PrCreationService, SystemPrRunner,
};
use crate::routes::error::{
    ApiError, api_error, deterministic_id, run_status_str, storage_error_to_http,
};
use crate::routes::team_gate::{self, TeamProjectContext};
use crate::scheduler::Scheduler;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use runlark_domain::{
    CommandId, CreateTeamRunCommand, Revision, Run, RunId, RunnerId, SessionId, Task, TaskId,
    TeamCommand, Timestamp,
};
use runlark_local::{
    BoardIntentParams, BoardStore, EnqueueTaskRunOutcome, EnqueueTaskRunParams, ProjectStore,
    RunQueries, SessionStore, TaskStore,
};
use serde::{Deserialize, Serialize};
use std::path::Path as StdPath;
use std::process::Command;
use uuid::Uuid;

pub use crate::routes::task_decisions::{
    AcceptTaskRequest, DecisionActionResponse, RejectTaskRequest, accept_task, reject_task,
};

#[derive(Debug, Deserialize, Default)]
pub struct StartTaskRequest {
    pub command_id: Option<String>,
    pub run_id: Option<String>,
    pub session_id: Option<String>,
    pub prompt: Option<String>,
    pub expected_revision: Option<u64>,
    pub runner_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StartTaskResponse {
    pub command_id: String,
    pub task: Task,
    pub run: Run,
    pub run_status: String,
    pub admitted: bool,
}

#[derive(Debug, Deserialize, Default)]
pub struct TaskPrRequest {
    pub command_id: Option<String>,
    pub run_id: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub base_branch: Option<String>,
    #[serde(default)]
    pub draft: bool,
    pub expected_revision: Option<u64>,
}

/// Starts (or enqueues in the 2-slot scheduler) a run for `task_id`, pinning `solo-default`
/// workflow digest on the `Run` and creating an isolated git worktree when admitted.
///
/// # Errors
/// Returns `404` if task missing, `409` on revision/active-run/payload conflict, `500` on I/O.
pub async fn start_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<StartTaskRequest>>,
) -> Result<(StatusCode, Json<StartTaskResponse>), ApiError> {
    let req = body.map(|b| b.0).unwrap_or_default();
    let task_id = TaskId::from(id);
    let explicit_cmd_id = req.command_id.filter(|s| !s.trim().is_empty());
    let has_explicit_cmd = explicit_cmd_id.is_some();
    let command_id = CommandId::from(
        explicit_cmd_id
            .clone()
            .unwrap_or_else(|| format!("cmd_start_{}", Uuid::new_v4().simple())),
    );
    let run_id = RunId::from(
        req.run_id
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| match &explicit_cmd_id {
                Some(cid) => deterministic_id("run", cid),
                None => format!("run_{}", Uuid::new_v4().simple()),
            }),
    );
    let session_id = SessionId::from(
        req.session_id
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| match &explicit_cmd_id {
                Some(cid) => deterministic_id("sess", cid),
                None => format!("sess_{}", Uuid::new_v4().simple()),
            }),
    );
    let workflow_digest = state
        .verification
        .latest_digest("solo-default")
        .map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "wasm_registry_error",
                e.to_string(),
            )
        })?;

    let (task_title, task_desc, repo_root) = load_task_start_meta(&state, &task_id)?;
    let base_sha = resolve_git_head(StdPath::new(&repo_root));
    let prompt = req.prompt.unwrap_or_else(|| {
        if task_desc.trim().is_empty() {
            task_title
        } else {
            format!("{task_title}\n\n{task_desc}")
        }
    });

    let now = Timestamp(AppState::current_timestamp_ms());
    if let Some((task, ctx)) = team_gate::resolve_task_team_context(&state, &task_id)? {
        return start_team_task(
            &state,
            &ctx,
            &task,
            TeamStartParams {
                command_id,
                has_explicit_cmd,
                run_id,
                runner_id: req.runner_id,
                expected_revision: req.expected_revision,
            },
            now,
        )
        .await;
    }

    let enqueue_params = EnqueueTaskRunParams {
        command_id: command_id.clone(),
        task_id: task_id.clone(),
        run_id: run_id.clone(),
        session_id,
        prompt,
        workflow_digest,
        base_sha,
        expected_revision: req.expected_revision.map(Revision),
    };

    let (status_code, persisted_run_id) = enqueue_start_tx(&state, &enqueue_params, &run_id, now)?;
    let scheduler = Scheduler::new(state.clone());
    let _ = scheduler
        .admit_once()
        .await
        .map_err(storage_error_to_http)?;
    state.notify_scheduler();

    load_started_task_response(&state, status_code, command_id, &task_id, &persisted_run_id)
}

struct TeamStartParams {
    command_id: CommandId,
    has_explicit_cmd: bool,
    run_id: RunId,
    runner_id: Option<String>,
    expected_revision: Option<u64>,
}

async fn start_team_task(
    state: &AppState,
    ctx: &TeamProjectContext,
    task: &Task,
    params: TeamStartParams,
    now: Timestamp,
) -> Result<(StatusCode, Json<StartTaskResponse>), ApiError> {
    let prior_rev = if params.expected_revision.is_none() && params.has_explicit_cmd {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        BoardStore::get_command_payload(&conn, &params.command_id)
            .map_err(storage_error_to_http)?
            .and_then(|json| serde_json::from_str::<TeamCommand>(&json).ok())
            .and_then(|c| match c {
                TeamCommand::CreateTeamRun(prev) => Some(prev.expected_revision),
                _ => None,
            })
    } else {
        None
    };
    let expected_revision =
        prior_rev.unwrap_or_else(|| Revision(params.expected_revision.unwrap_or(task.revision.0)));
    let chosen_runner = params
        .runner_id
        .filter(|s| !s.trim().is_empty())
        .map(RunnerId::from)
        .or_else(|| ctx.runner_id.clone());
    let cmd = TeamCommand::CreateTeamRun(CreateTeamRunCommand {
        command_id: params.command_id.clone(),
        run_id: params.run_id,
        project_id: task.project_id.clone(),
        task_id: task.id.clone(),
        runner_id: chosen_runner,
        expected_revision,
        created_at: now,
    });
    let res = team_gate::submit_team_command_checked(state, ctx, &cmd).await?;
    if params.has_explicit_cmd && prior_rev.is_none() {
        record_acked_team_start_intent(
            state,
            &params.command_id,
            &res.aggregate_id,
            &cmd,
            expected_revision,
            now,
        )?;
    }
    let status_code = if res.idempotent_replay {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    let created_run_id = RunId::from(res.aggregate_id);
    state.notify_scheduler();
    state.notify_runner();
    load_started_task_response(
        state,
        status_code,
        params.command_id,
        &task.id,
        &created_run_id,
    )
}

fn record_acked_team_start_intent(
    state: &AppState,
    command_id: &CommandId,
    aggregate_id: &str,
    cmd: &TeamCommand,
    expected_revision: Revision,
    now: Timestamp,
) -> Result<(), ApiError> {
    let payload_json = serde_json::to_string(&cmd.canonical_for_hash()).map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialization_error",
            e.to_string(),
        )
    })?;
    let mut conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    RunQueries::persist_board_intent(
        &mut conn,
        BoardIntentParams {
            command_id,
            aggregate_type: "run",
            aggregate_id,
            command_type: "create_team_run",
            payload_json: &payload_json,
            expected_revision,
        },
        now,
    )
    .map_err(storage_error_to_http)?;
    RunQueries::update_board_intent_status(&conn, command_id, "acked", None, now)
        .map_err(storage_error_to_http)?;
    Ok(())
}

fn load_task_start_meta(
    state: &AppState,
    task_id: &TaskId,
) -> Result<(String, String, String), ApiError> {
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
    let project = ProjectStore::get_project(&conn, &task.project_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "project_not_found",
                format!("Project not found: {}", task.project_id),
            )
        })?;
    Ok((task.title, task.description, project.repo_root))
}

fn enqueue_start_tx(
    state: &AppState,
    enqueue_params: &EnqueueTaskRunParams,
    fallback_run_id: &RunId,
    now: Timestamp,
) -> Result<(StatusCode, RunId), ApiError> {
    let mut conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    match RunQueries::enqueue_task_run(&mut conn, enqueue_params, now)
        .map_err(storage_error_to_http)?
    {
        EnqueueTaskRunOutcome::Enqueued {
            session,
            run,
            events,
            ..
        } => {
            state.publish_events(&session.id.0, &events);
            Ok((StatusCode::CREATED, run.id))
        }
        EnqueueTaskRunOutcome::AlreadyExists { run, .. } => Ok((
            StatusCode::OK,
            run.map_or_else(|| fallback_run_id.clone(), |r| r.id),
        )),
    }
}

fn load_started_task_response(
    state: &AppState,
    status_code: StatusCode,
    command_id: CommandId,
    task_id: &TaskId,
    run_id: &RunId,
) -> Result<(StatusCode, Json<StartTaskResponse>), ApiError> {
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
    let run = SessionStore::get_run(&conn, run_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "run_not_found",
                format!("Run not found: {run_id}"),
            )
        })?;

    Ok((
        status_code,
        Json(StartTaskResponse {
            command_id: command_id.0,
            admitted: run.status.consumes_execution_slot(),
            run_status: run_status_str(run.status).to_string(),
            task,
            run,
        }),
    ))
}

/// Publishes a PR for an accepted task run using `PrCreationService`.
///
/// # Errors
/// Returns `404` if task/run missing, `409` if human acceptance or bindings fail,
/// or `502` if `git`/`gh` fails.
pub async fn create_task_pr(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<TaskPrRequest>>,
) -> Result<(StatusCode, Json<CreatePrOutcome>), ApiError> {
    let req = body.map(|b| b.0).unwrap_or_default();
    let task_id = TaskId::from(id);
    let task_team_ctx = team_gate::resolve_task_team_context(&state, &task_id)?;
    if let Some((_, ref ctx)) = task_team_ctx {
        team_gate::verify_server_reachable_and_synced(&state, ctx).await?;
    }
    let (run_id, resolved_run) = {
        let conn = state.get_connection().map_err(|e| {
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
        let rid = req
            .run_id
            .filter(|s| !s.trim().is_empty())
            .map(RunId::from)
            .or(task.latest_run_id)
            .or(task.active_run_id)
            .ok_or_else(|| {
                api_error(
                    StatusCode::CONFLICT,
                    "missing_run",
                    "task has no run to publish PR for",
                )
            })?;
        let run = SessionStore::get_run(&conn, &rid).map_err(storage_error_to_http)?;
        if let Some(ref r) = run
            && r.task_id.as_ref() != Some(&task_id)
        {
            return Err(api_error(
                StatusCode::CONFLICT,
                "run_task_mismatch",
                format!("Run {rid} does not belong to task {task_id}"),
            ));
        }
        (rid, run)
    };
    if task_team_ctx.is_none()
        && let Some(ref run) = resolved_run
        && let Some(ctx) = team_gate::resolve_run_team_context(&state, run)?
    {
        team_gate::verify_server_reachable_and_synced(&state, &ctx).await?;
    }

    let pr_req = CreatePrRequest {
        command_id: CommandId::from(
            req.command_id
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| format!("cmd_pr_{}", Uuid::new_v4().simple())),
        ),
        run_id,
        title: req.title,
        body: req.body,
        base_branch: req.base_branch,
        draft: req.draft,
        expected_revision: req.expected_revision,
    };

    let runner = SystemPrRunner::default();
    match PrCreationService::create_pr(&state, &runner, &pr_req).await {
        Ok(outcome @ CreatePrOutcome::Created { replayed: true, .. }) => {
            Ok((StatusCode::OK, Json(outcome)))
        }
        Ok(
            outcome @ CreatePrOutcome::Created {
                replayed: false, ..
            },
        ) => Ok((StatusCode::CREATED, Json(outcome))),
        Ok(outcome @ CreatePrOutcome::Unknown { .. }) => Ok((StatusCode::ACCEPTED, Json(outcome))),
        Err(GitPrError::Storage(se)) => Err(storage_error_to_http(se)),
        Err(GitPrError::NotFound(msg)) => Err(api_error(StatusCode::NOT_FOUND, "not_found", msg)),
        Err(GitPrError::Conflict(msg)) => Err(api_error(
            StatusCode::CONFLICT,
            "pr_precondition_failed",
            msg,
        )),
        Err(GitPrError::CommandFailed(msg)) => {
            Err(api_error(StatusCode::BAD_GATEWAY, "pr_command_failed", msg))
        }
        Err(GitPrError::UnknownOutcome(msg)) => Err(api_error(
            StatusCode::BAD_GATEWAY,
            "pr_outcome_unknown",
            msg,
        )),
    }
}

pub(super) fn resolve_git_head(dir: &StdPath) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!sha.is_empty()).then_some(sha)
}
