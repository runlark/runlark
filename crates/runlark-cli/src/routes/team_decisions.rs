use crate::git_pr::compute_worktree_diff;
use crate::routes::error::{ApiError, api_error, storage_error_to_http, team_server_error_to_http};
use crate::routes::team_gate::{TeamProjectContext, submit_team_command_checked};
use crate::state::AppState;
use crate::team_sync::TeamSyncWorker;
use axum::http::StatusCode;
use runlark_domain::{
    ArtifactDigest, ArtifactKind, AttachArtifactCommand, CommandId, DecisionId, DecisionKind,
    PolicyDigest, ProjectId, RecordDecisionCommand, Revision, Run, RunId, RunStatus, Task, TaskId,
    TeamCommand, Timestamp, WorkflowStage,
};
use runlark_local::{ArtifactCasStore, ProjectStore, SessionStore, TaskStore};
use std::path::Path as StdPath;
use uuid::Uuid;

/// Ensures local evidence artifacts for `run_id` are uploaded to the team server CAS and linked
/// to `run_id` on the server before submitting a bound `RecordDecision` command.
///
/// # Errors
/// Returns `503` if server unreachable, `409` on conflict, or `500` on local CAS error.
pub async fn sync_run_evidence_to_server(
    state: &AppState,
    ctx: &TeamProjectContext,
    run_id: &RunId,
    now: Timestamp,
) -> Result<(), ApiError> {
    let local_artifacts = {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        ArtifactCasStore::list_artifacts_for_run(&conn, run_id).map_err(storage_error_to_http)?
    };
    let cas = state.cas_store();
    let worker = TeamSyncWorker::new(state.clone());

    for (meta, label) in local_artifacts {
        if meta.kind != ArtifactKind::EvidenceBundle || !cas.has_blob(&meta.digest_sha256) {
            continue;
        }
        let bytes = cas
            .get_bytes(&meta.digest_sha256)
            .map_err(storage_error_to_http)?;
        worker
            .upload_artifact(
                &ctx.server_url,
                &ctx.bearer_token,
                &ctx.workspace_id,
                &meta,
                &bytes,
            )
            .await
            .map_err(team_server_error_to_http)?;

        let command_id =
            TeamSyncWorker::deterministic_attach_command_id(run_id, &meta.digest_sha256, &label);
        let attach_cmd = TeamCommand::AttachArtifact(AttachArtifactCommand {
            command_id,
            run_id: run_id.clone(),
            digest_sha256: meta.digest_sha256,
            kind: meta.kind,
            media_type: meta.media_type,
            size_bytes: meta.size_bytes,
            label,
            created_at: now,
        });
        let _ = worker
            .submit_command_and_sync(
                &ctx.server_url,
                &ctx.bearer_token,
                &ctx.workspace_id,
                &attach_cmd,
            )
            .await
            .map_err(team_server_error_to_http)?;
    }
    Ok(())
}

/// Inputs required to submit a bound team `Accept` decision to `runlark-server`.
pub struct TeamAcceptParams {
    pub command_id: CommandId,
    pub decision_id: DecisionId,
    pub actor: String,
    pub comment: Option<String>,
    pub expected_revision: Option<u64>,
    pub plan_version: Option<u32>,
    pub head_sha: Option<String>,
    pub base_sha: Option<String>,
    pub policy_digest: Option<String>,
    pub evidence_digest: Option<String>,
}

/// Submits an authoritative `DecisionKind::Accept` for a `TeamManaged` task, waiting for server
/// confirmation and never committing locally when offline.
///
/// # Errors
/// Returns `503` if server unreachable, `409` on binding/revision conflict, `403` on RBAC denial.
pub async fn execute_team_accept(
    state: &AppState,
    ctx: &TeamProjectContext,
    task: &Task,
    params: TeamAcceptParams,
    now: Timestamp,
) -> Result<(Task, Run), ApiError> {
    let run_id = resolve_decision_run_id(task, "task has no run to accept")?;
    sync_run_evidence_to_server(state, ctx, &run_id, now).await?;

    let (run, project_policy, evidence_digest) =
        load_run_policy_and_evidence(state, &run_id, &task.project_id, false)?;
    let attached_evidence = {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        ArtifactCasStore::list_artifacts_for_run(&conn, &run.id)
            .map_err(storage_error_to_http)?
            .into_iter()
            .rev()
            .find(|(m, label)| m.kind == ArtifactKind::EvidenceBundle && label == "evidence_bundle")
            .map(|(m, _)| m.digest_sha256)
    };
    let (live_head, live_base) = resolve_live_shas(&run)?;
    let chosen_evidence = match params.evidence_digest {
        Some(raw) => ArtifactDigest::parse(raw)
            .map_err(|e| api_error(StatusCode::BAD_REQUEST, "invalid_digest", e.to_string()))?,
        None => attached_evidence
            .or(Some(evidence_digest))
            .filter(|d| !d.as_str().chars().all(|c| c == '0'))
            .ok_or_else(|| {
                api_error(
                    StatusCode::CONFLICT,
                    "evidence_required",
                    "verification evidence_bundle artifact is required before acceptance",
                )
            })?,
    };
    let cmd = TeamCommand::RecordDecision(RecordDecisionCommand {
        command_id: params.command_id,
        decision_id: params.decision_id,
        task_id: task.id.clone(),
        run_id: run.id.clone(),
        kind: DecisionKind::Accept,
        actor: params.actor,
        plan_version: params.plan_version.unwrap_or(task.plan_version),
        head_sha: params.head_sha.unwrap_or(live_head),
        base_sha: params.base_sha.unwrap_or(live_base),
        policy_digest: params
            .policy_digest
            .map_or(project_policy, PolicyDigest::from),
        evidence_digest: chosen_evidence,
        comment: params.comment,
        expected_revision: Revision(params.expected_revision.unwrap_or(task.revision.0)),
        created_at: now,
    });
    submit_team_command_checked(state, ctx, &cmd).await?;
    reload_task_and_run(state, &task.id, &run_id)
}

/// Parameters for rejecting / requesting changes on a `TeamManaged` task.
#[derive(Debug, Clone)]
pub struct TeamRejectParams {
    pub command_id: CommandId,
    pub decision_id: DecisionId,
    pub actor: String,
    pub reason: String,
    pub expected_revision: Option<u64>,
}

/// Submits an authoritative `DecisionKind::RequestChanges` for a `TeamManaged` task, waiting
/// for server confirmation and never committing locally when offline.
///
/// # Errors
/// Returns `503` if server unreachable, `409` on revision/state conflict, `403` on RBAC denial.
pub async fn execute_team_reject(
    state: &AppState,
    ctx: &TeamProjectContext,
    task: &Task,
    params: TeamRejectParams,
    now: Timestamp,
) -> Result<(Task, Run), ApiError> {
    let run_id = resolve_decision_run_id(task, "task has no run to reject")?;
    let (run, project_policy, evidence_digest) =
        load_run_policy_and_evidence(state, &run_id, &task.project_id, false)?;
    let cmd = TeamCommand::RecordDecision(RecordDecisionCommand {
        command_id: params.command_id,
        decision_id: params.decision_id,
        task_id: task.id.clone(),
        run_id: run.id.clone(),
        kind: DecisionKind::RequestChanges,
        actor: params.actor,
        plan_version: task.plan_version,
        head_sha: run.head_sha.unwrap_or_default(),
        base_sha: run.base_sha.unwrap_or_default(),
        policy_digest: project_policy,
        evidence_digest,
        comment: Some(params.reason),
        expected_revision: Revision(params.expected_revision.unwrap_or(task.revision.0)),
        created_at: now,
    });
    submit_team_command_checked(state, ctx, &cmd).await?;
    reload_task_and_run(state, &task.id, &run_id)
}

/// Validates that `task` currently has an active/latest non-terminal run in `WorkflowStage::Plan`
/// with bound commit SHAs eligible for `DecisionKind::ApprovePlan`.
///
/// # Errors
/// Returns `409` if no active run exists or the run cannot receive a plan approval decision.
pub fn validate_team_plan_approval_eligible(
    state: &AppState,
    task: &Task,
) -> Result<(Run, PolicyDigest, ArtifactDigest), ApiError> {
    let run_id = resolve_decision_run_id(
        task,
        "cannot approve plan in TeamManaged mode without an active run",
    )?;
    let (run, policy_digest, evidence_digest) =
        load_run_policy_and_evidence(state, &run_id, &task.project_id, true)?;
    if run.status.is_terminal() || run.status == RunStatus::CancelRequested {
        return Err(api_error(
            StatusCode::CONFLICT,
            "run_not_approvable",
            format!(
                "cannot approve plan on run {} with status {:?}",
                run.id, run.status
            ),
        ));
    }
    if run.workflow_stage != WorkflowStage::Plan {
        return Err(api_error(
            StatusCode::CONFLICT,
            "invalid_transition",
            format!(
                "cannot approve plan when run {} is in stage {:?}",
                run.id, run.workflow_stage
            ),
        ));
    }
    let (head, base) = resolve_live_shas(&run)?;
    if head.trim().is_empty() || base.trim().is_empty() {
        return Err(api_error(
            StatusCode::CONFLICT,
            "missing_decision_binding",
            "cannot approve team plan when run head_sha or base_sha is missing",
        ));
    }
    Ok((run, policy_digest, evidence_digest))
}

/// Records `DecisionKind::ApprovePlan` on `runlark-server` bound to `task.plan_version`.
///
/// # Errors
/// Returns `409` if ineligible or binding conflicted, `503` if server unreachable.
pub async fn execute_team_approve_plan(
    state: &AppState,
    ctx: &TeamProjectContext,
    task: &Task,
    actor: String,
    now: Timestamp,
) -> Result<Task, ApiError> {
    let (run, policy_digest, evidence_digest) = validate_team_plan_approval_eligible(state, task)?;
    let (head_sha, base_sha) = resolve_live_shas(&run)?;
    let cmd = TeamCommand::RecordDecision(RecordDecisionCommand {
        command_id: CommandId::from(format!("cmd_app_plan_{}", Uuid::new_v4().simple())),
        decision_id: DecisionId::from(format!("dec_app_plan_{}", Uuid::new_v4().simple())),
        task_id: task.id.clone(),
        run_id: run.id.clone(),
        kind: DecisionKind::ApprovePlan,
        actor,
        plan_version: task.plan_version,
        head_sha,
        base_sha,
        policy_digest,
        evidence_digest,
        comment: None,
        expected_revision: task.revision,
        created_at: now,
    });
    submit_team_command_checked(state, ctx, &cmd).await?;
    let (updated, _) = reload_task_and_run(state, &task.id, &run.id)?;
    Ok(updated)
}

fn resolve_decision_run_id(task: &Task, missing_msg: &str) -> Result<RunId, ApiError> {
    task.active_run_id
        .clone()
        .or_else(|| task.latest_run_id.clone())
        .ok_or_else(|| api_error(StatusCode::CONFLICT, "missing_run", missing_msg))
}

fn load_run_policy_and_evidence(
    state: &AppState,
    run_id: &RunId,
    project_id: &ProjectId,
    prefer_plan_artifact: bool,
) -> Result<(Run, PolicyDigest, ArtifactDigest), ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
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
    let project = ProjectStore::get_project(&conn, project_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "project_not_found",
                format!("Project not found: {project_id}"),
            )
        })?;
    let artifacts =
        ArtifactCasStore::list_artifacts_for_run(&conn, &run.id).map_err(storage_error_to_http)?;
    let chosen = artifacts
        .iter()
        .rev()
        .find(|(m, _)| {
            if prefer_plan_artifact {
                matches!(m.kind, ArtifactKind::Plan | ArtifactKind::EvidenceBundle)
            } else {
                m.kind == ArtifactKind::EvidenceBundle
            }
        })
        .map_or_else(
            || ArtifactDigest::parse("0".repeat(64)).unwrap_or_else(|_| unreachable!()),
            |(m, _)| m.digest_sha256.clone(),
        );
    let pinned_policy = run.policy_digest.clone().unwrap_or(project.policy_digest);
    Ok((run, pinned_policy, chosen))
}

fn resolve_live_shas(run: &Run) -> Result<(String, String), ApiError> {
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

fn reload_task_and_run(
    state: &AppState,
    task_id: &TaskId,
    run_id: &RunId,
) -> Result<(Task, Run), ApiError> {
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
    Ok((task, run))
}
