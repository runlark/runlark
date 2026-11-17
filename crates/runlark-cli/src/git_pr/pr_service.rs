use super::{
    CreatePrOutcome, CreatePrRequest, GitPrError, PrCommandError, PrCommandRunner,
    compute_worktree_diff, git_stdout,
};
use crate::state::AppState;
use crate::worktree::WorktreeManager;
use runlark_domain::{
    ArtifactKind, ArtifactRef, DecisionBinding, PolicyDigest, Timestamp, TransitionError,
    verify_decision_binding,
};
use runlark_local::{
    ArtifactCasStore, BoardIntentOutcome, BoardIntentParams, DecisionStore, ProjectStore,
    RunQueries, SessionStore, StorageError, TaskStore,
};
use std::path::Path;

/// Coordinates bound human-acceptance verification, intent persistence, and PR creation.
pub struct PrCreationService;

struct LoadedPrContext {
    run: runlark_domain::Run,
    task: Option<runlark_domain::Task>,
    project: Option<runlark_domain::Project>,
    decision: runlark_domain::Decision,
    evidence_ref: ArtifactRef,
}

struct FinalizePrParams<'a> {
    state: &'a AppState,
    req: &'a CreatePrRequest,
    ctx: &'a LoadedPrContext,
    branch_name: &'a str,
    base_branch: &'a str,
    current_head_sha: &'a str,
    current_base_sha: &'a str,
    pr_url: String,
}

impl PrCreationService {
    /// Verifies bound `Decision::Accept`, persists intent before external push/PR, executes
    /// `git push` and `gh pr create`, and stores the PR receipt in CAS.
    ///
    /// # Errors
    /// Returns [`GitPrError`] on payload/binding conflicts (`409`), missing entities (`404`),
    /// or clean git/gh failures. Ambiguous failures return `Ok(CreatePrOutcome::Unknown { .. })`.
    pub async fn create_pr(
        state: &AppState,
        runner: &dyn PrCommandRunner,
        req: &CreatePrRequest,
    ) -> Result<CreatePrOutcome, GitPrError> {
        let now = Timestamp(AppState::current_timestamp_ms());
        let payload_json = serde_json::to_string(req).map_err(StorageError::Serialization)?;

        let ctx = match Self::prepare_pr_context(state, req, &payload_json, now)? {
            Ok(ctx) => ctx,
            Err(early_outcome) => return Ok(early_outcome),
        };

        let worktree_path = Path::new(&ctx.run.worktree_path);
        let (current_head_sha, current_base_sha, branch_name) =
            Self::resolve_git_refs(&ctx, worktree_path)?;
        let current_plan_version = ctx
            .task
            .as_ref()
            .map_or(ctx.run.plan_version, |t| t.plan_version);
        let current_policy_digest: PolicyDigest = ctx
            .project
            .as_ref()
            .map(|p| p.policy_digest.clone())
            .or_else(|| ctx.run.policy_digest.clone())
            .unwrap_or_else(|| ctx.decision.policy_digest.clone());

        let binding = DecisionBinding {
            plan_version: current_plan_version,
            head_sha: &current_head_sha,
            base_sha: &current_base_sha,
            policy_digest: &current_policy_digest,
            evidence_digest: &ctx.evidence_ref.digest_sha256,
        };
        if let Err(err) = verify_decision_binding(&ctx.decision, &binding) {
            let conn = state.get_connection()?;
            let _ = RunQueries::update_board_intent_status(
                &conn,
                &req.command_id,
                "failed",
                Some(&err.to_string()),
                Timestamp(AppState::current_timestamp_ms()),
            );
            return Err(GitPrError::Conflict(err.to_string()));
        }

        {
            let conn = state.get_connection()?;
            RunQueries::update_board_intent_status(
                &conn,
                &req.command_id,
                "dispatched",
                None,
                Timestamp(AppState::current_timestamp_ms()),
            )?;
        }

        let (pr_url, base_branch) = match Self::push_and_create_pr(
            state,
            runner,
            req,
            &ctx,
            worktree_path,
            &branch_name,
            &current_head_sha,
        )
        .await?
        {
            Ok(pair) => pair,
            Err(early_outcome) => return Ok(early_outcome),
        };

        Self::finalize_pr_success(FinalizePrParams {
            state,
            req,
            ctx: &ctx,
            branch_name: &branch_name,
            base_branch: &base_branch,
            current_head_sha: &current_head_sha,
            current_base_sha: &current_base_sha,
            pr_url,
        })
    }

    async fn push_and_create_pr(
        state: &AppState,
        runner: &dyn PrCommandRunner,
        req: &CreatePrRequest,
        ctx: &LoadedPrContext,
        worktree_path: &Path,
        branch_name: &str,
        current_head_sha: &str,
    ) -> Result<Result<(String, String), CreatePrOutcome>, GitPrError> {
        let push_args = ["push", "-u", "origin", branch_name];
        if let Err(err) = runner.run_git(worktree_path, &push_args).await {
            return Ok(Err(Self::handle_runner_error(state, req, err)?));
        }

        let base_branch = req
            .base_branch
            .clone()
            .or_else(|| ctx.project.as_ref().map(|p| p.default_branch.clone()))
            .unwrap_or_else(|| "main".to_string());
        let pr_title = req.title.clone().unwrap_or_else(|| {
            ctx.task
                .as_ref()
                .map_or_else(|| format!("Runlark {}", req.run_id), |t| t.title.clone())
        });
        let pr_body = req.body.clone().unwrap_or_else(|| {
            format!(
                "Runlark run `{}` accepted by `{}` (head: `{}`, evidence: `{}`).",
                req.run_id, ctx.decision.actor, current_head_sha, ctx.evidence_ref.digest_sha256
            )
        });

        let mut gh_args = vec![
            "pr",
            "create",
            "--title",
            pr_title.as_str(),
            "--body",
            pr_body.as_str(),
            "--base",
            base_branch.as_str(),
            "--head",
            branch_name,
        ];
        if req.draft {
            gh_args.push("--draft");
        }

        let gh_out = match runner.run_gh(worktree_path, &gh_args).await {
            Ok(out) => out,
            Err(err) => return Ok(Err(Self::handle_runner_error(state, req, err)?)),
        };

        let pr_url = extract_pr_url(&gh_out.stdout).ok_or_else(|| {
            let msg = format!(
                "gh pr create succeeded without returning a PR URL: {}",
                gh_out.stdout.trim()
            );
            match state.get_connection() {
                Ok(conn) => {
                    if let Err(err) = RunQueries::update_board_intent_status(
                        &conn,
                        &req.command_id,
                        "dispatched",
                        Some(&msg),
                        Timestamp(AppState::current_timestamp_ms()),
                    ) {
                        tracing::error!("failed to record PR unknown outcome intent: {err}");
                    }
                }
                Err(err) => {
                    tracing::error!(
                        "failed to open connection for PR unknown outcome intent: {err}"
                    );
                }
            }
            GitPrError::UnknownOutcome(msg)
        })?;
        Ok(Ok((pr_url, base_branch)))
    }

    fn prepare_pr_context(
        state: &AppState,
        req: &CreatePrRequest,
        payload_json: &str,
        now: Timestamp,
    ) -> Result<Result<LoadedPrContext, CreatePrOutcome>, GitPrError> {
        let mut conn = state.get_connection()?;
        let expected_rev = runlark_domain::Revision(req.expected_revision.unwrap_or(0));
        match RunQueries::persist_board_intent(
            &mut conn,
            BoardIntentParams {
                command_id: &req.command_id,
                aggregate_type: "run",
                aggregate_id: req.run_id.as_ref(),
                command_type: "create_pr",
                payload_json,
                expected_revision: expected_rev,
            },
            now,
        )? {
            BoardIntentOutcome::AlreadyExists { record } => {
                if record.status == "acked" {
                    let existing_run = SessionStore::get_run(&conn, &req.run_id)?
                        .ok_or_else(|| GitPrError::NotFound(format!("Run {}", req.run_id)))?;
                    return Ok(Err(CreatePrOutcome::Created {
                        command_id: req.command_id.0.clone(),
                        run_id: req.run_id.0.clone(),
                        branch: WorktreeManager::branch_name(&req.run_id),
                        pr_url: existing_run.pr_url.unwrap_or_default(),
                        receipt_digest: String::new(),
                        replayed: true,
                    }));
                }
                if record.status == "dispatched" {
                    return Ok(Err(CreatePrOutcome::Unknown {
                        command_id: req.command_id.0.clone(),
                        run_id: req.run_id.0.clone(),
                        reason: "prior PR publish outcome is still unknown".to_string(),
                    }));
                }
            }
            BoardIntentOutcome::Inserted => {}
        }

        let run = SessionStore::get_run(&conn, &req.run_id)?
            .ok_or_else(|| StorageError::RunNotFound(req.run_id.clone()))?;
        let task = match &run.task_id {
            Some(tid) => TaskStore::get_task(&conn, tid)?,
            None => None,
        };
        let project = match &run.project_id {
            Some(pid) => ProjectStore::get_project(&conn, pid)?,
            None => None,
        };
        let decision =
            DecisionStore::latest_accept_for_run(&conn, &req.run_id)?.ok_or_else(|| {
                GitPrError::Conflict(TransitionError::AcceptanceDecisionRequired.to_string())
            })?;
        let cas = state.cas_store();
        let evidence_ref = ArtifactCasStore::list_artifacts_for_run(&conn, &req.run_id)?
            .into_iter()
            .rev()
            .find(|(meta, label)| {
                meta.kind == ArtifactKind::EvidenceBundle && label == "evidence_bundle"
            })
            .map(|(meta, _)| meta)
            .ok_or_else(|| {
                GitPrError::Conflict(
                    "evidence_bundle CAS artifact is required before PR creation".to_string(),
                )
            })?;
        let _ = cas.get_bytes(&evidence_ref.digest_sha256)?;

        Ok(Ok(LoadedPrContext {
            run,
            task,
            project,
            decision,
            evidence_ref,
        }))
    }

    fn resolve_git_refs(
        ctx: &LoadedPrContext,
        worktree_path: &Path,
    ) -> Result<(String, String, String), GitPrError> {
        if worktree_path.is_dir() && git_stdout(worktree_path, &["rev-parse", "HEAD"]).is_ok() {
            let snap = compute_worktree_diff(worktree_path, ctx.run.base_sha.as_deref())?;
            return Ok((snap.head_sha, snap.base_sha, snap.branch_name));
        }
        let head = ctx
            .run
            .head_sha
            .clone()
            .ok_or_else(|| GitPrError::Conflict("run missing head_sha".to_string()))?;
        let base = ctx
            .run
            .base_sha
            .clone()
            .ok_or_else(|| GitPrError::Conflict("run missing base_sha".to_string()))?;
        Ok((head, base, WorktreeManager::branch_name(&ctx.run.id)))
    }

    fn finalize_pr_success(params: FinalizePrParams<'_>) -> Result<CreatePrOutcome, GitPrError> {
        let FinalizePrParams {
            state,
            req,
            ctx,
            branch_name,
            base_branch,
            current_head_sha,
            current_base_sha,
            pr_url,
        } = params;
        let done_now = Timestamp(AppState::current_timestamp_ms());
        let mut conn = state.get_connection()?;
        let receipt_json = serde_json::to_vec_pretty(&serde_json::json!({
            "run_id": req.run_id.0,
            "branch": branch_name,
            "base_branch": base_branch,
            "head_sha": current_head_sha,
            "base_sha": current_base_sha,
            "decision_id": ctx.decision.id.0,
            "evidence_digest": ctx.evidence_ref.digest_sha256.as_str(),
            "pr_url": pr_url,
            "created_at": done_now.0,
        }))
        .map_err(StorageError::Serialization)?;
        let cas = state.cas_store();
        let receipt_ref = cas.put_bytes(
            &conn,
            ArtifactKind::EvidenceBundle,
            "application/json",
            &receipt_json,
            done_now,
        )?;
        ArtifactCasStore::link_to_run(
            &conn,
            &req.run_id,
            &receipt_ref.digest_sha256,
            "pr_receipt",
            done_now,
        )?;
        RunQueries::update_run_and_task_pr_url(
            &mut conn,
            &req.run_id,
            ctx.task.as_ref().map(|t| &t.id),
            &pr_url,
            done_now,
        )?;
        RunQueries::update_board_intent_status(&conn, &req.command_id, "acked", None, done_now)?;

        Ok(CreatePrOutcome::Created {
            command_id: req.command_id.0.clone(),
            run_id: req.run_id.0.clone(),
            branch: branch_name.to_string(),
            pr_url,
            receipt_digest: receipt_ref.digest_sha256.to_string(),
            replayed: false,
        })
    }

    fn handle_runner_error(
        state: &AppState,
        req: &CreatePrRequest,
        err: PrCommandError,
    ) -> Result<CreatePrOutcome, GitPrError> {
        let now = Timestamp(AppState::current_timestamp_ms());
        let conn = state.get_connection()?;
        match err {
            PrCommandError::Unknown(reason) => {
                RunQueries::update_board_intent_status(
                    &conn,
                    &req.command_id,
                    "dispatched",
                    Some(&reason),
                    now,
                )?;
                Ok(CreatePrOutcome::Unknown {
                    command_id: req.command_id.0.clone(),
                    run_id: req.run_id.0.clone(),
                    reason,
                })
            }
            PrCommandError::Failed(msg) => {
                RunQueries::update_board_intent_status(
                    &conn,
                    &req.command_id,
                    "failed",
                    Some(&msg),
                    now,
                )?;
                Err(GitPrError::CommandFailed(msg))
            }
        }
    }
}

fn extract_pr_url(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with("http://") || line.starts_with("https://"))
        .map(str::to_string)
}
