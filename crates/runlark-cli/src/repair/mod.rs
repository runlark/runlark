mod stages;
mod team_report;

use crate::git_pr::{GitDiffSnapshot, compute_worktree_diff};
use crate::state::AppState;
use runlark_domain::{
    ArtifactDigest, BudgetBlockReason, Revision, Run, RunId, RunStatus, TaskStage, Timestamp,
    WorkflowStage, advance_run_stage,
};
use runlark_local::{
    EventStore, ProjectStore, SessionStore, StorageError, StoredEvent, TaskStore, open_sqlite,
    with_immediate_tx,
};
use runlark_wasm::{
    CheckEvidence, ValidationInput, ValidationReport, ValidationVerdict, WasmError,
    WorkflowStage as WasmWorkflowStage,
};
use serde::Serialize;
use std::path::Path;
use thiserror::Error;

/// Options controlling a verification + bounded repair pass.
#[derive(Debug, Clone, Default)]
pub struct VerifyPipelineOptions {
    pub package_name: Option<String>,
    pub check_command: Option<String>,
    pub extra_checks: Vec<CheckEvidence>,
    pub elapsed_ms_override: Option<u64>,
}

/// Outcome of driving verification and bounded repair on a run.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum VerifyPipelineOutcome {
    PassedForAcceptance {
        run_id: String,
        workflow_stage: WorkflowStage,
        run_status: RunStatus,
        head_sha: String,
        base_sha: String,
        diff_digest: ArtifactDigest,
        verification_log_digest: ArtifactDigest,
        wasm_validation_digest: ArtifactDigest,
        evidence_digest: ArtifactDigest,
        report: ValidationReport,
    },
    RepairScheduled {
        run_id: String,
        repair_attempt: u32,
        reason: String,
        diff_digest: ArtifactDigest,
        verification_log_digest: ArtifactDigest,
        wasm_validation_digest: ArtifactDigest,
        report: ValidationReport,
    },
    BudgetHalted {
        run_id: String,
        block_reason: BudgetBlockReason,
        reason: String,
        diff_digest: ArtifactDigest,
        verification_log_digest: ArtifactDigest,
        wasm_validation_digest: ArtifactDigest,
        report: ValidationReport,
    },
}

/// Errors raised by [`WorkflowPipeline`].
#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("wasm validation error: {0}")]
    Wasm(#[from] WasmError),

    #[error("domain transition error: {0}")]
    Transition(#[from] runlark_domain::TransitionError),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("team fencing or sync error: {0}")]
    TeamFencing(String),
}

struct VerifyContext {
    run: Run,
    task: Option<runlark_domain::Task>,
    project: Option<runlark_domain::Project>,
    team_lease: Option<team_report::TeamRunLease>,
}

/// Executes verification, stores CAS artifacts, and drives bounded repair or human acceptance.
pub struct WorkflowPipeline;

impl WorkflowPipeline {
    /// Handles automatic stage progression when a DSH turn ends (`Completed`) on a task-backed run.
    ///
    /// # Errors
    /// Returns [`PipelineError`] if loading, verification, or persistence fails.
    pub async fn on_dsh_turn_ended(
        state: &AppState,
        run_id: &RunId,
        now: Timestamp,
    ) -> Result<Option<VerifyPipelineOutcome>, PipelineError> {
        let current_stage = {
            let conn = state.get_connection()?;
            let Some(run) = SessionStore::get_run(&conn, run_id)? else {
                return Ok(None);
            };
            if run.task_id.is_none() || run.status.is_terminal() {
                return Ok(None);
            }
            run.workflow_stage
        };

        match current_stage {
            WorkflowStage::Clarification => {
                Self::advance_stage_with_task(state, run_id, WorkflowStage::Plan, now).await?;
                Ok(None)
            }
            WorkflowStage::Plan => {
                Self::advance_stage_with_task(state, run_id, WorkflowStage::Implementation, now)
                    .await?;
                Ok(None)
            }
            WorkflowStage::Implementation | WorkflowStage::Verification => {
                let outcome =
                    Self::verify_and_advance(state, run_id, VerifyPipelineOptions::default(), now)
                        .await?;
                Ok(Some(outcome))
            }
            WorkflowStage::HumanAcceptance => Ok(None),
        }
    }

    /// Advances a run's `WorkflowStage`, mirrors it onto the parent `Task`, and reports or
    /// offline-buffers the fenced state update for `TeamManaged` runs.
    ///
    /// # Errors
    /// Returns [`PipelineError`] on missing entity, fencing conflict, or invalid transition.
    pub async fn advance_stage_with_task(
        state: &AppState,
        run_id: &RunId,
        target_stage: WorkflowStage,
        now: Timestamp,
    ) -> Result<Run, PipelineError> {
        let mut conn = state.get_connection()?;
        let (new_run, stored, lease, task_before): (
            Run,
            Vec<StoredEvent>,
            Option<team_report::TeamRunLease>,
            Option<team_report::TaskBeforeMeta>,
        ) = with_immediate_tx(&mut conn, |tx| {
            let run = SessionStore::get_run(tx, run_id)?
                .ok_or_else(|| StorageError::RunNotFound(run_id.clone()))?;
            let mut task_opt = match &run.task_id {
                Some(tid) => TaskStore::get_task(tx, tid)?,
                None => None,
            };
            let lease = team_report::resolve_team_run_lease(tx, &run, task_opt.as_ref())?;
            let task_before = task_opt.as_ref().map(team_report::TaskBeforeMeta::from);

            let outcome = advance_run_stage(&run, target_stage, now)?;
            SessionStore::save_run(tx, &outcome.new_run)?;

            if let Some(ref mut task) = task_opt {
                let expected = task.revision;
                task.workflow_stage = target_stage;
                if target_stage == WorkflowStage::HumanAcceptance {
                    task.stage = TaskStage::Acceptance;
                }
                task.revision = Revision(expected.0 + 1);
                task.updated_at = now;
                TaskStore::update_task_with_revision(tx, task, expected)?;
            }

            let mut stored = Vec::with_capacity(outcome.events.len());
            for ev in &outcome.events {
                stored.push(EventStore::append_domain_event(tx, run_id, ev, now)?);
            }
            Ok::<_, PipelineError>((outcome.new_run, stored, lease, task_before))
        })?;
        drop(conn);
        state.publish_events(&new_run.session_id.0, &stored);
        if new_run.status == RunStatus::WaitingForApproval {
            state.notify_scheduler();
        }
        if let Some(ref lease) = lease {
            team_report::report_stage_advance(
                state,
                lease,
                &new_run,
                task_before.as_ref(),
                target_stage,
                now,
            )
            .await?;
        }
        Ok(new_run)
    }

    /// Runs git diffing, optional project check command, and digest-pinned Wasmtime validation,
    /// stores `Diff`, `VerificationLog`, `WasmValidation`, and `EvidenceBundle` in CAS, and
    /// transitions either to `HumanAcceptance` (`WaitingForApproval`) or bounded repair / `Failed`.
    ///
    /// # Errors
    /// Returns [`PipelineError`] if storage, CAS persistence, team fencing, or Wasmtime fails.
    pub async fn verify_and_advance(
        state: &AppState,
        run_id: &RunId,
        options: VerifyPipelineOptions,
        now: Timestamp,
    ) -> Result<VerifyPipelineOutcome, PipelineError> {
        let ctx = Self::load_verify_context(state, run_id)?;
        let (run, task_opt, project_opt, team_lease) =
            (ctx.run, ctx.task, ctx.project, ctx.team_lease);
        let started_at_implementation = run.workflow_stage == WorkflowStage::Implementation;
        let task_before = task_opt.as_ref().map(team_report::TaskBeforeMeta::from);
        let (diff_snap, checks, check_logs) =
            Self::collect_diff_and_checks(state, &run, &options).await?;

        let (report, verification_passed) = Self::run_wasm_validation(
            state,
            &run,
            task_opt.as_ref(),
            &diff_snap,
            &checks,
            options.package_name.as_deref(),
        )?;

        let mut conn = open_sqlite(&state.db_path)?;
        let (outcome, buffered_events) = with_immediate_tx(&mut conn, |tx| {
            let mut events_out = Vec::new();
            let (diff_ref, verif_log_ref, wasm_ref) =
                Self::persist_verification_artifacts(&mut stages::VerifyPersistInput {
                    state,
                    conn: tx,
                    run: &run,
                    diff_snap: &diff_snap,
                    checks: &checks,
                    check_logs: &check_logs,
                    report: &report,
                    events_out: &mut events_out,
                    now,
                })?;
            let working_run =
                Self::transition_to_verification(tx, &run, &diff_snap, &mut events_out, now)?;

            let outcome = if verification_passed {
                Self::finalize_pass_for_acceptance(stages::PassFinalizeInput {
                    state,
                    conn: tx,
                    working_run: &working_run,
                    task_opt: task_opt.as_ref(),
                    project_opt: project_opt.as_ref(),
                    diff_snap: &diff_snap,
                    refs: (diff_ref, verif_log_ref, wasm_ref),
                    report,
                    events_out: &mut events_out,
                    now,
                })?
            } else {
                Self::handle_verification_failure(stages::FailureHandleInput {
                    conn: tx,
                    working_run: &working_run,
                    task_opt: task_opt.as_ref(),
                    checks: &checks,
                    refs: (diff_ref, verif_log_ref, wasm_ref),
                    report,
                    elapsed_ms_override: options.elapsed_ms_override,
                    events_out: &mut events_out,
                    now,
                })?
            };
            Ok::<_, PipelineError>((outcome, events_out))
        })?;
        drop(conn);

        for (session_id, stored) in buffered_events {
            state.publish_events(&session_id, std::slice::from_ref(&stored));
        }
        match &outcome {
            VerifyPipelineOutcome::PassedForAcceptance { .. }
            | VerifyPipelineOutcome::BudgetHalted { .. } => state.notify_scheduler(),
            VerifyPipelineOutcome::RepairScheduled { .. } => state.notify_outbox(),
        }

        if let Some(ref lease) = team_lease {
            team_report::report_verification_outcome(
                state,
                lease,
                &team_report::VerificationReportInput {
                    run_id,
                    started_at_implementation,
                    head_sha: &diff_snap.head_sha,
                    base_sha: &diff_snap.base_sha,
                    task_before: task_before.as_ref(),
                    outcome: &outcome,
                    now,
                },
            )
            .await?;
        }
        Ok(outcome)
    }

    fn run_wasm_validation(
        state: &AppState,
        run: &Run,
        task_opt: Option<&runlark_domain::Task>,
        diff_snap: &GitDiffSnapshot,
        checks: &[CheckEvidence],
        package_name: Option<&str>,
    ) -> Result<(ValidationReport, bool), PipelineError> {
        let pkg = package_name.unwrap_or("solo-default");
        let val_input = ValidationInput {
            task_id: task_opt.map_or_else(|| run.session_id.0.clone(), |t| t.id.0.clone()),
            run_id: run.id.0.clone(),
            stage: WasmWorkflowStage::Validate,
            changed_files: diff_snap.changed_files.clone(),
            checks: checks.to_vec(),
            diff_bytes: diff_snap.diff_bytes,
            repair_attempt: run.repair_attempts,
        };
        let report = state.verification.verify_run_with_digest(
            &run.id,
            run.workflow_digest.as_ref(),
            pkg,
            &val_input,
        )?;
        let passed = checks.iter().all(|c| c.passed) && report.verdict == ValidationVerdict::Pass;
        Ok((report, passed))
    }

    fn load_verify_context(
        state: &AppState,
        run_id: &RunId,
    ) -> Result<VerifyContext, PipelineError> {
        let conn = state.get_connection()?;
        let run = SessionStore::get_run(&conn, run_id)?
            .ok_or_else(|| StorageError::RunNotFound(run_id.clone()))?;
        let task = match &run.task_id {
            Some(tid) => TaskStore::get_task(&conn, tid)?,
            None => None,
        };
        let project = match &run.project_id {
            Some(pid) => ProjectStore::get_project(&conn, pid)?,
            None => None,
        };
        let team_lease = team_report::resolve_team_run_lease(&conn, &run, task.as_ref())?;
        Ok(VerifyContext {
            run,
            task,
            project,
            team_lease,
        })
    }

    async fn collect_diff_and_checks(
        state: &AppState,
        run: &Run,
        options: &VerifyPipelineOptions,
    ) -> Result<(GitDiffSnapshot, Vec<CheckEvidence>, Vec<serde_json::Value>), PipelineError> {
        let worktree = run.worktree_path.clone();
        let base_hint = run.base_sha.clone();
        let check_cmd = options
            .check_command
            .clone()
            .or_else(|| state.project_check_command.clone());

        let (diff_snap, check_capture) = tokio::task::spawn_blocking(move || {
            let wt_path = Path::new(&worktree);
            let snap = compute_worktree_diff(wt_path, base_hint.as_deref()).map_err(|e| {
                StorageError::InvalidState(format!(
                    "worktree git diff failed for {}: {e}",
                    wt_path.display()
                ))
            })?;
            let check = check_cmd
                .as_deref()
                .map(|cmd| stages::run_project_check(wt_path, cmd));
            Ok::<_, StorageError>((snap, check))
        })
        .await
        .map_err(|e| StorageError::InvalidState(format!("verification worker panicked: {e}")))??;

        let mut checks = options.extra_checks.clone();
        let mut check_logs = Vec::new();
        if let Some(captured) = check_capture {
            check_logs.push(serde_json::json!({
                "name": captured.evidence.name,
                "passed": captured.evidence.passed,
                "exit_code": captured.evidence.exit_code,
                "stdout": captured.stdout,
                "stderr": captured.stderr,
            }));
            checks.push(captured.evidence);
        }
        if checks.is_empty() {
            checks.push(CheckEvidence {
                name: "git-diff-present".to_string(),
                passed: true,
                exit_code: 0,
            });
        }
        Ok((diff_snap, checks, check_logs))
    }
}
