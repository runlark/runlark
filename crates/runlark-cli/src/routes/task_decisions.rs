use crate::git_pr::compute_worktree_diff;
use crate::routes::error::{ApiError, api_error, deterministic_id, storage_error_to_http};
use crate::routes::team_gate::{self, TeamAcceptParams};
use crate::scheduler::Scheduler;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use runlark_domain::{
    ArtifactDigest, ArtifactKind, BoardCommand, CommandId, DecisionBinding, DecisionId,
    DecisionKind, PolicyDigest, RecordDecisionCommand, Revision, Run, RunId, Task, TaskId,
    Timestamp, verify_decision_binding,
};
use runlark_local::{
    ArtifactCasStore, BoardCommandOutcome, BoardStore, DecisionStore, ProjectStore, SessionStore,
    TaskStore,
};
use serde::{Deserialize, Serialize};
use std::path::Path as StdPath;
use uuid::Uuid;

#[derive(Debug, Deserialize, Default)]
pub struct AcceptTaskRequest {
    pub command_id: Option<String>,
    pub actor: Option<String>,
    pub comment: Option<String>,
    pub expected_revision: Option<u64>,
    pub plan_version: Option<u32>,
    pub head_sha: Option<String>,
    pub base_sha: Option<String>,
    pub policy_digest: Option<String>,
    pub evidence_digest: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RejectTaskRequest {
    pub command_id: Option<String>,
    pub actor: Option<String>,
    pub reason: String,
    pub expected_revision: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct DecisionActionResponse {
    pub decision_id: String,
    pub task: Task,
    pub run: Run,
}

/// Records human `Decision::Accept` bound to exact `plan_version`, `head_sha`, `base_sha`,
/// `policy_digest`, and `evidence_digest`, moving the task to `Done` and run to `Completed`.
///
/// # Errors
/// Returns `404` if task/run missing, `409` if not in `Acceptance`, missing evidence, or
/// bound fields drifted.
pub async fn accept_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<AcceptTaskRequest>>,
) -> Result<Json<DecisionActionResponse>, ApiError> {
    let req = body.map(|b| b.0).unwrap_or_default();
    let task_id = TaskId::from(id);
    let now = Timestamp(AppState::current_timestamp_ms());
    let (command_id, decision_id, has_explicit_cmd) =
        resolve_decision_ids("cmd_acc", "dec_acc", req.command_id.as_deref());
    let actor = normalize_actor(req.actor.as_deref());

    if let Some((task, ctx)) = team_gate::resolve_task_team_context(&state, &task_id)? {
        let (updated_task, updated_run) = team_gate::execute_team_accept(
            &state,
            &ctx,
            &task,
            TeamAcceptParams {
                command_id,
                decision_id: decision_id.clone(),
                actor,
                comment: req.comment,
                expected_revision: req.expected_revision,
                plan_version: req.plan_version,
                head_sha: req.head_sha,
                base_sha: req.base_sha,
                policy_digest: req.policy_digest,
                evidence_digest: req.evidence_digest,
            },
            now,
        )
        .await?;
        return Ok(Json(DecisionActionResponse {
            decision_id: decision_id.0,
            task: updated_task,
            run: updated_run,
        }));
    }

    let mut conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    if has_explicit_cmd && let Some(prev) = load_prior_decision(&conn, &command_id)? {
        return replay_accept(&mut conn, &prev, &task_id, &command_id, &req, &actor, now);
    }

    let run_id = apply_fresh_accept(
        &mut conn,
        &task_id,
        &command_id,
        &decision_id,
        &req,
        &actor,
        now,
    )?;
    state.notify_scheduler();
    tokio::task::yield_now().await;
    load_decision_action_response(&conn, decision_id, &task_id, &run_id)
}

fn replay_accept(
    conn: &mut runlark_local::Connection,
    prev: &RecordDecisionCommand,
    task_id: &TaskId,
    command_id: &CommandId,
    req: &AcceptTaskRequest,
    actor: &str,
    now: Timestamp,
) -> Result<Json<DecisionActionResponse>, ApiError> {
    let evidence_digest = match &req.evidence_digest {
        Some(raw) => ArtifactDigest::parse(raw.clone())
            .map_err(|e| api_error(StatusCode::BAD_REQUEST, "invalid_digest", e.to_string()))?,
        None => prev.evidence_digest.clone(),
    };
    let replay_cmd = BoardCommand::RecordDecision(RecordDecisionCommand {
        command_id: command_id.clone(),
        decision_id: prev.decision_id.clone(),
        task_id: task_id.clone(),
        run_id: prev.run_id.clone(),
        kind: DecisionKind::Accept,
        actor: actor.to_string(),
        plan_version: req.plan_version.unwrap_or(prev.plan_version),
        head_sha: req
            .head_sha
            .clone()
            .unwrap_or_else(|| prev.head_sha.clone()),
        base_sha: req
            .base_sha
            .clone()
            .unwrap_or_else(|| prev.base_sha.clone()),
        policy_digest: req.policy_digest.as_ref().map_or_else(
            || prev.policy_digest.clone(),
            |p| PolicyDigest::from(p.as_str()),
        ),
        evidence_digest,
        comment: req.comment.clone(),
        expected_revision: Revision(req.expected_revision.unwrap_or(prev.expected_revision.0)),
        created_at: now,
    });
    BoardStore::apply_board_command(conn, &replay_cmd, now).map_err(storage_error_to_http)?;
    load_decision_action_response(conn, prev.decision_id.clone(), task_id, &prev.run_id)
}

fn apply_fresh_accept(
    conn: &mut runlark_local::Connection,
    task_id: &TaskId,
    command_id: &CommandId,
    decision_id: &DecisionId,
    req: &AcceptTaskRequest,
    actor: &str,
    now: Timestamp,
) -> Result<RunId, ApiError> {
    let (task, run, project) = load_decision_entities(conn, task_id, "accept")?;
    if run.plan_version != task.plan_version {
        return Err(api_error(
            StatusCode::CONFLICT,
            "decision_binding_mismatch",
            format!(
                "task plan version ({}) changed since run verification ({})",
                task.plan_version, run.plan_version
            ),
        ));
    }
    let (live_head, live_base) = live_git_shas(&run)?;
    let evidence_ref = ArtifactCasStore::list_artifacts_for_run(conn, &run.id)
        .map_err(storage_error_to_http)?
        .into_iter()
        .rev()
        .find(|(meta, label)| {
            meta.kind == ArtifactKind::EvidenceBundle && label == "evidence_bundle"
        })
        .map(|(meta, _)| meta.digest_sha256)
        .ok_or_else(|| {
            api_error(
                StatusCode::CONFLICT,
                "evidence_required",
                "verification evidence_bundle artifact is required before acceptance",
            )
        })?;

    let expected_plan_version = req.plan_version.unwrap_or(task.plan_version);
    let expected_head = req.head_sha.clone().unwrap_or_else(|| live_head.clone());
    let expected_base = req.base_sha.clone().unwrap_or_else(|| live_base.clone());
    let expected_policy = req.policy_digest.as_ref().map_or_else(
        || project.policy_digest.clone(),
        |p| PolicyDigest::from(p.as_str()),
    );
    let expected_evidence = match &req.evidence_digest {
        Some(raw) => ArtifactDigest::parse(raw.clone())
            .map_err(|e| api_error(StatusCode::BAD_REQUEST, "invalid_digest", e.to_string()))?,
        None => evidence_ref.clone(),
    };

    let probe_decision = runlark_domain::Decision {
        id: decision_id.clone(),
        task_id: task.id.clone(),
        run_id: run.id.clone(),
        kind: DecisionKind::Accept,
        actor: actor.to_string(),
        plan_version: expected_plan_version,
        head_sha: expected_head.clone(),
        base_sha: expected_base.clone(),
        policy_digest: expected_policy.clone(),
        evidence_digest: expected_evidence.clone(),
        comment: req.comment.clone(),
        created_at: now,
    };
    let live_binding = DecisionBinding {
        plan_version: task.plan_version,
        head_sha: &live_head,
        base_sha: &live_base,
        policy_digest: &project.policy_digest,
        evidence_digest: &evidence_ref,
    };
    verify_decision_binding(&probe_decision, &live_binding).map_err(|e| {
        api_error(
            StatusCode::CONFLICT,
            "decision_binding_mismatch",
            e.to_string(),
        )
    })?;

    let expected_revision = Revision(req.expected_revision.unwrap_or(task.revision.0));
    let cmd = BoardCommand::RecordDecision(RecordDecisionCommand {
        command_id: command_id.clone(),
        decision_id: decision_id.clone(),
        task_id: task.id,
        run_id: run.id.clone(),
        kind: DecisionKind::Accept,
        actor: actor.to_string(),
        plan_version: expected_plan_version,
        head_sha: expected_head,
        base_sha: expected_base,
        policy_digest: expected_policy,
        evidence_digest: expected_evidence,
        comment: req.comment.clone(),
        expected_revision,
        created_at: now,
    });
    BoardStore::apply_board_command(conn, &cmd, now).map_err(storage_error_to_http)?;
    Ok(run.id)
}

/// Records human `Decision::RequestChanges`, returning the task to `InProgress`/`Implementation`
/// and queuing the run back into the 2-slot scheduler.
///
/// # Errors
/// Returns `400` if reason blank, `404` if task/run missing, `409` on binding/revision conflict.
pub async fn reject_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<RejectTaskRequest>,
) -> Result<Json<DecisionActionResponse>, ApiError> {
    if req.reason.trim().is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_reject",
            "reason is required",
        ));
    }

    let task_id = TaskId::from(id);
    let now = Timestamp(AppState::current_timestamp_ms());
    let (command_id, decision_id, has_explicit_cmd) =
        resolve_decision_ids("cmd_rej", "dec_rej", req.command_id.as_deref());
    let actor = normalize_actor(req.actor.as_deref());

    if let Some((task, ctx)) = team_gate::resolve_task_team_context(&state, &task_id)? {
        let (updated_task, updated_run) = team_gate::execute_team_reject(
            &state,
            &ctx,
            &task,
            team_gate::TeamRejectParams {
                command_id,
                decision_id: decision_id.clone(),
                actor,
                reason: req.reason,
                expected_revision: req.expected_revision,
            },
            now,
        )
        .await?;
        return Ok(Json(DecisionActionResponse {
            decision_id: decision_id.0,
            task: updated_task,
            run: updated_run,
        }));
    }

    let (replayed, run_id, decision_id) = {
        let mut conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        if has_explicit_cmd && let Some(prev) = load_prior_decision(&conn, &command_id)? {
            replay_reject(&mut conn, &prev, &task_id, &command_id, &req, &actor, now)?
        } else {
            let run_id = apply_fresh_reject(
                &mut conn,
                &task_id,
                &command_id,
                &decision_id,
                &req,
                &actor,
                now,
            )?;
            (false, run_id, decision_id)
        }
    };

    if !replayed {
        let scheduler = Scheduler::new(state.clone());
        let _ = scheduler
            .admit_once()
            .await
            .map_err(storage_error_to_http)?;
        state.notify_scheduler();
    }

    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    load_decision_action_response(&conn, decision_id, &task_id, &run_id)
}

fn replay_reject(
    conn: &mut runlark_local::Connection,
    prev: &RecordDecisionCommand,
    task_id: &TaskId,
    command_id: &CommandId,
    req: &RejectTaskRequest,
    actor: &str,
    now: Timestamp,
) -> Result<(bool, RunId, DecisionId), ApiError> {
    let replay_cmd = BoardCommand::RecordDecision(RecordDecisionCommand {
        command_id: command_id.clone(),
        decision_id: prev.decision_id.clone(),
        task_id: task_id.clone(),
        run_id: prev.run_id.clone(),
        kind: DecisionKind::RequestChanges,
        actor: actor.to_string(),
        plan_version: prev.plan_version,
        head_sha: prev.head_sha.clone(),
        base_sha: prev.base_sha.clone(),
        policy_digest: prev.policy_digest.clone(),
        evidence_digest: prev.evidence_digest.clone(),
        comment: Some(req.reason.clone()),
        expected_revision: Revision(req.expected_revision.unwrap_or(prev.expected_revision.0)),
        created_at: now,
    });
    let outcome =
        BoardStore::apply_board_command(conn, &replay_cmd, now).map_err(storage_error_to_http)?;
    Ok((
        matches!(outcome, BoardCommandOutcome::AlreadyExists { .. }),
        prev.run_id.clone(),
        prev.decision_id.clone(),
    ))
}

fn apply_fresh_reject(
    conn: &mut runlark_local::Connection,
    task_id: &TaskId,
    command_id: &CommandId,
    decision_id: &DecisionId,
    req: &RejectTaskRequest,
    actor: &str,
    now: Timestamp,
) -> Result<RunId, ApiError> {
    let (task, run, project) = load_decision_entities(conn, task_id, "reject")?;
    let evidence_digest = ArtifactCasStore::list_artifacts_for_run(conn, &run.id)
        .map_err(storage_error_to_http)?
        .into_iter()
        .rev()
        .find(|(m, _)| m.kind == ArtifactKind::EvidenceBundle)
        .map_or_else(zero_digest, |(m, _)| m.digest_sha256);

    let head_sha = run
        .head_sha
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "HEAD".to_string());
    let base_sha = run
        .base_sha
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "HEAD".to_string());

    let expected_revision = Revision(req.expected_revision.unwrap_or(task.revision.0));
    let cmd = BoardCommand::RecordDecision(RecordDecisionCommand {
        command_id: command_id.clone(),
        decision_id: decision_id.clone(),
        task_id: task.id,
        run_id: run.id.clone(),
        kind: DecisionKind::RequestChanges,
        actor: actor.to_string(),
        plan_version: task.plan_version,
        head_sha,
        base_sha,
        policy_digest: project.policy_digest,
        evidence_digest,
        comment: Some(req.reason.clone()),
        expected_revision,
        created_at: now,
    });
    BoardStore::apply_board_command(conn, &cmd, now).map_err(storage_error_to_http)?;
    Ok(run.id)
}

fn resolve_decision_ids(
    cmd_prefix: &str,
    dec_prefix: &str,
    raw_command_id: Option<&str>,
) -> (CommandId, DecisionId, bool) {
    let explicit = raw_command_id.map(str::trim).filter(|s| !s.is_empty());
    let cmd_id = CommandId::from(match explicit {
        Some(cid) => cid.to_string(),
        None => format!("{cmd_prefix}_{}", Uuid::new_v4().simple()),
    });
    let dec_id = DecisionId::from(match explicit {
        Some(cid) => deterministic_id(dec_prefix, cid),
        None => format!("{dec_prefix}_{}", Uuid::new_v4().simple()),
    });
    (cmd_id, dec_id, explicit.is_some())
}

fn normalize_actor(raw: Option<&str>) -> String {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("human")
        .to_string()
}

fn load_prior_decision(
    conn: &runlark_local::Connection,
    command_id: &CommandId,
) -> Result<Option<RecordDecisionCommand>, ApiError> {
    let Some(prev_json) =
        BoardStore::get_command_payload(conn, command_id).map_err(storage_error_to_http)?
    else {
        return Ok(None);
    };
    Ok(
        match serde_json::from_str::<BoardCommand>(&prev_json).ok() {
            Some(BoardCommand::RecordDecision(prev)) => Some(prev),
            _ => None,
        },
    )
}

fn zero_digest() -> ArtifactDigest {
    ArtifactDigest::parse("0".repeat(64)).unwrap_or_else(|_| unreachable!())
}

fn load_decision_entities(
    conn: &runlark_local::Connection,
    task_id: &TaskId,
    action: &str,
) -> Result<(Task, Run, runlark_domain::Project), ApiError> {
    let task = TaskStore::get_task(conn, task_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "task_not_found",
                format!("Task not found: {task_id}"),
            )
        })?;
    let run_id = task
        .active_run_id
        .clone()
        .or_else(|| task.latest_run_id.clone())
        .ok_or_else(|| {
            api_error(
                StatusCode::CONFLICT,
                "missing_run",
                format!("task has no run to {action}"),
            )
        })?;
    let run = SessionStore::get_run(conn, &run_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "run_not_found",
                format!("Run not found: {run_id}"),
            )
        })?;
    let project = ProjectStore::get_project(conn, &task.project_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "project_not_found",
                format!("Project not found: {}", task.project_id),
            )
        })?;
    Ok((task, run, project))
}

fn live_git_shas(run: &Run) -> Result<(String, String), ApiError> {
    let wt_path = StdPath::new(&run.worktree_path);
    if wt_path.is_dir() && crate::routes::task_actions::resolve_git_head(wt_path).is_some() {
        let snap = compute_worktree_diff(wt_path, run.base_sha.as_deref())
            .map_err(|e| api_error(StatusCode::CONFLICT, "git_diff_error", e.to_string()))?;
        return Ok((snap.head_sha, snap.base_sha));
    }
    Ok((
        run.head_sha.clone().unwrap_or_default(),
        run.base_sha.clone().unwrap_or_default(),
    ))
}

fn load_decision_action_response(
    conn: &runlark_local::Connection,
    decision_id: DecisionId,
    task_id: &TaskId,
    run_id: &RunId,
) -> Result<Json<DecisionActionResponse>, ApiError> {
    let updated_task = TaskStore::get_task(conn, task_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "task_not_found",
                format!("Task not found: {task_id}"),
            )
        })?;
    let updated_run = SessionStore::get_run(conn, run_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "run_not_found",
                format!("Run not found: {run_id}"),
            )
        })?;
    DecisionStore::get_decision(conn, &decision_id).map_err(storage_error_to_http)?;

    Ok(Json(DecisionActionResponse {
        decision_id: decision_id.0,
        task: updated_task,
        run: updated_run,
    }))
}
