use super::{PipelineError, VerifyPipelineOutcome, WorkflowPipeline};
use crate::git_pr::{GitDiffSnapshot, store_diff_in_cas};
use crate::state::AppState;
use runlark_domain::{
    ArtifactDigest, ArtifactKind, ArtifactRef, BudgetBlockReason, CommandId, DomainEvent,
    RepairScheduleOutcome, Revision, Run, RunId, SendPromptCommand, TaskStage, Timestamp,
    WorkflowStage, advance_run_stage, schedule_repair,
};
use runlark_local::{
    ArtifactCasStore, AttemptStore, EventStore, RunQueries, SessionStore, StorageError,
    StoredEvent, TaskStore,
};
use runlark_wasm::{CheckEvidence, ValidationReport};
use std::path::Path;
use std::process::Command;

pub(super) type BufferedEvents = Vec<(String, StoredEvent)>;

pub(super) struct CheckCommandCapture {
    pub(super) evidence: CheckEvidence,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

pub(super) struct VerifyPersistInput<'a> {
    pub(super) state: &'a AppState,
    pub(super) conn: &'a runlark_local::Connection,
    pub(super) run: &'a Run,
    pub(super) diff_snap: &'a GitDiffSnapshot,
    pub(super) checks: &'a [CheckEvidence],
    pub(super) check_logs: &'a [serde_json::Value],
    pub(super) report: &'a ValidationReport,
    pub(super) events_out: &'a mut BufferedEvents,
    pub(super) now: Timestamp,
}

pub(super) struct PassFinalizeInput<'a> {
    pub(super) state: &'a AppState,
    pub(super) conn: &'a runlark_local::Connection,
    pub(super) working_run: &'a Run,
    pub(super) task_opt: Option<&'a runlark_domain::Task>,
    pub(super) project_opt: Option<&'a runlark_domain::Project>,
    pub(super) diff_snap: &'a GitDiffSnapshot,
    pub(super) refs: (ArtifactRef, ArtifactRef, ArtifactRef),
    pub(super) report: ValidationReport,
    pub(super) is_team_managed: bool,
    pub(super) events_out: &'a mut BufferedEvents,
    pub(super) now: Timestamp,
}

pub(super) struct FailureHandleInput<'a> {
    pub(super) conn: &'a runlark_local::Connection,
    pub(super) working_run: &'a Run,
    pub(super) task_opt: Option<&'a runlark_domain::Task>,
    pub(super) checks: &'a [CheckEvidence],
    pub(super) refs: (ArtifactRef, ArtifactRef, ArtifactRef),
    pub(super) report: ValidationReport,
    pub(super) elapsed_ms_override: Option<u64>,
    pub(super) is_team_managed: bool,
    pub(super) events_out: &'a mut BufferedEvents,
    pub(super) now: Timestamp,
}

impl WorkflowPipeline {
    pub(super) fn persist_verification_artifacts(
        input: &mut VerifyPersistInput<'_>,
    ) -> Result<(ArtifactRef, ArtifactRef, ArtifactRef), PipelineError> {
        let VerifyPersistInput {
            state,
            conn,
            run,
            diff_snap,
            checks,
            check_logs,
            report,
            events_out,
            now,
        } = input;
        let state = *state;
        let conn = *conn;
        let run = *run;
        let diff_snap = *diff_snap;
        let checks = *checks;
        let check_logs = *check_logs;
        let report = *report;
        let now = *now;
        let cas = state.cas_store();
        let diff_ref = store_diff_in_cas(&cas, conn, &run.id, diff_snap, now)?;
        let verif_log_bytes = serde_json::to_vec_pretty(&serde_json::json!({
            "run_id": run.id.0,
            "base_sha": diff_snap.base_sha,
            "head_sha": diff_snap.head_sha,
            "changed_files": diff_snap.changed_files,
            "checks": checks,
            "check_outputs": check_logs,
            "diff_digest": diff_ref.digest_sha256.as_str(),
            "created_at": now.0,
        }))
        .map_err(StorageError::Serialization)?;
        let verif_log_ref = cas.put_bytes(
            conn,
            ArtifactKind::VerificationLog,
            "application/json",
            &verif_log_bytes,
            now,
        )?;
        ArtifactCasStore::link_to_run(
            conn,
            &run.id,
            &verif_log_ref.digest_sha256,
            "verification_log",
            now,
        )?;

        let wasm_bytes = serde_json::to_vec_pretty(report).map_err(StorageError::Serialization)?;
        let wasm_ref = cas.put_bytes(
            conn,
            ArtifactKind::WasmValidation,
            "application/json",
            &wasm_bytes,
            now,
        )?;
        ArtifactCasStore::link_to_run(
            conn,
            &run.id,
            &wasm_ref.digest_sha256,
            "wasm_validation",
            now,
        )?;

        emit_artifact_attached(
            conn,
            &run.id,
            &diff_ref,
            "diff",
            &run.session_id.0,
            events_out,
            now,
        )?;
        emit_artifact_attached(
            conn,
            &run.id,
            &verif_log_ref,
            "verification_log",
            &run.session_id.0,
            events_out,
            now,
        )?;
        emit_artifact_attached(
            conn,
            &run.id,
            &wasm_ref,
            "wasm_validation",
            &run.session_id.0,
            events_out,
            now,
        )?;
        Ok((diff_ref, verif_log_ref, wasm_ref))
    }

    pub(super) fn transition_to_verification(
        conn: &runlark_local::Connection,
        run: &Run,
        diff_snap: &GitDiffSnapshot,
        events_out: &mut BufferedEvents,
        now: Timestamp,
    ) -> Result<Run, PipelineError> {
        let mut working_run = SessionStore::get_run(conn, &run.id)?
            .ok_or_else(|| StorageError::RunNotFound(run.id.clone()))?;
        working_run.head_sha = Some(diff_snap.head_sha.clone());
        working_run.base_sha = Some(diff_snap.base_sha.clone());
        if working_run.workflow_stage == WorkflowStage::Implementation {
            let verif_stage = advance_run_stage(&working_run, WorkflowStage::Verification, now)?;
            working_run = verif_stage.new_run;
            working_run.head_sha = Some(diff_snap.head_sha.clone());
            working_run.base_sha = Some(diff_snap.base_sha.clone());
            for ev in &verif_stage.events {
                let stored = EventStore::append_domain_event(conn, &working_run.id, ev, now)?;
                events_out.push((working_run.session_id.0.clone(), stored));
            }
        }
        SessionStore::save_run(conn, &working_run)?;
        Ok(working_run)
    }

    pub(super) fn finalize_pass_for_acceptance(
        input: PassFinalizeInput<'_>,
    ) -> Result<VerifyPipelineOutcome, PipelineError> {
        let PassFinalizeInput {
            state,
            conn,
            working_run,
            task_opt,
            project_opt,
            diff_snap,
            refs: (diff_ref, verif_log_ref, wasm_ref),
            report,
            is_team_managed,
            events_out,
            now,
        } = input;
        let cas = state.cas_store();
        let evidence_bytes = serde_json::to_vec_pretty(&serde_json::json!({
            "run_id": working_run.id.0,
            "task_id": working_run.task_id.as_ref().map(|t| t.0.as_str()),
            "plan_version": task_opt.map_or(working_run.plan_version, |t| t.plan_version),
            "head_sha": diff_snap.head_sha,
            "base_sha": diff_snap.base_sha,
            "policy_digest": project_opt.map(|p| p.policy_digest.as_ref()).or(working_run.policy_digest.as_ref().map(AsRef::as_ref)),
            "workflow_digest": working_run.workflow_digest.as_ref().map(ArtifactDigest::as_str),
            "diff_digest": diff_ref.digest_sha256.as_str(),
            "verification_log_digest": verif_log_ref.digest_sha256.as_str(),
            "wasm_validation_digest": wasm_ref.digest_sha256.as_str(),
            "verdict": "pass",
            "created_at": now.0,
        }))
        .map_err(StorageError::Serialization)?;
        let evidence_ref = cas.put_bytes(
            conn,
            ArtifactKind::EvidenceBundle,
            "application/json",
            &evidence_bytes,
            now,
        )?;
        ArtifactCasStore::link_to_run(
            conn,
            &working_run.id,
            &evidence_ref.digest_sha256,
            "evidence_bundle",
            now,
        )?;
        emit_artifact_attached(
            conn,
            &working_run.id,
            &evidence_ref,
            "evidence_bundle",
            &working_run.session_id.0,
            events_out,
            now,
        )?;

        let accept_stage = advance_run_stage(working_run, WorkflowStage::HumanAcceptance, now)?;
        SessionStore::save_run(conn, &accept_stage.new_run)?;
        if let Some(tid) = &accept_stage.new_run.task_id
            && let Some(mut task) = TaskStore::get_task(conn, tid)?
        {
            let expected = task.revision;
            let prev_stage = task.stage;
            task.stage = TaskStage::Acceptance;
            task.workflow_stage = WorkflowStage::HumanAcceptance;
            task.head_ref = Some(diff_snap.branch_name.clone());
            if !is_team_managed {
                task.revision = Revision(expected.0 + 1);
            }
            task.updated_at = now;
            TaskStore::update_task_with_revision(conn, &task, expected)?;
            if prev_stage != TaskStage::Acceptance {
                let ev = DomainEvent::TaskStageChanged {
                    task_id: task.id,
                    from: prev_stage,
                    to: TaskStage::Acceptance,
                    timestamp: now,
                };
                let stored = EventStore::append_domain_event(conn, &working_run.id, &ev, now)?;
                events_out.push((working_run.session_id.0.clone(), stored));
            }
        }
        for ev in &accept_stage.events {
            let stored = EventStore::append_domain_event(conn, &working_run.id, ev, now)?;
            events_out.push((working_run.session_id.0.clone(), stored));
        }

        Ok(VerifyPipelineOutcome::PassedForAcceptance {
            run_id: accept_stage.new_run.id.0,
            workflow_stage: accept_stage.new_run.workflow_stage,
            run_status: accept_stage.new_run.status,
            head_sha: diff_snap.head_sha.clone(),
            base_sha: diff_snap.base_sha.clone(),
            diff_digest: diff_ref.digest_sha256,
            verification_log_digest: verif_log_ref.digest_sha256,
            wasm_validation_digest: wasm_ref.digest_sha256,
            evidence_digest: evidence_ref.digest_sha256,
            report,
        })
    }

    pub(super) fn handle_verification_failure(
        input: FailureHandleInput<'_>,
    ) -> Result<VerifyPipelineOutcome, PipelineError> {
        let FailureHandleInput {
            conn,
            working_run,
            task_opt,
            checks,
            refs,
            report,
            elapsed_ms_override,
            is_team_managed,
            events_out,
            now,
        } = input;
        let attempts = AttemptStore::list_attempts_for_run(conn, &working_run.id)?;
        let policy = task_opt
            .and_then(|t| t.budget_override.clone())
            .unwrap_or_else(|| working_run.budget_policy.clone());
        let elapsed_ms =
            elapsed_ms_override.unwrap_or_else(|| now.0.saturating_sub(working_run.created_at.0));

        let mut failure_notes: Vec<String> = checks
            .iter()
            .filter(|c| !c.passed)
            .map(|c| format!("check `{}` failed (exit {})", c.name, c.exit_code))
            .collect();
        failure_notes.extend(report.findings.iter().cloned());
        if failure_notes.is_empty() {
            failure_notes.push(report.summary.clone());
        }
        let reason_text = failure_notes.join("; ");

        match schedule_repair(
            working_run,
            &attempts,
            &policy,
            elapsed_ms,
            reason_text.clone(),
            now,
        )? {
            RepairScheduleOutcome::Scheduled { new_run, events } => {
                Self::persist_scheduled_repair(
                    conn,
                    &new_run,
                    &events,
                    &reason_text,
                    is_team_managed,
                    events_out,
                    now,
                )?;
                Ok(VerifyPipelineOutcome::RepairScheduled {
                    run_id: new_run.id.0,
                    repair_attempt: new_run.repair_attempts,
                    reason: reason_text,
                    diff_digest: refs.0.digest_sha256,
                    verification_log_digest: refs.1.digest_sha256,
                    wasm_validation_digest: refs.2.digest_sha256,
                    report,
                })
            }
            RepairScheduleOutcome::Halted {
                new_run,
                reason: block_reason,
                events,
            } => {
                let block_msg = Self::persist_halted_repair(
                    conn,
                    &new_run,
                    &block_reason,
                    &events,
                    events_out,
                    now,
                )?;
                Ok(VerifyPipelineOutcome::BudgetHalted {
                    run_id: new_run.id.0,
                    block_reason,
                    reason: block_msg,
                    diff_digest: refs.0.digest_sha256,
                    verification_log_digest: refs.1.digest_sha256,
                    wasm_validation_digest: refs.2.digest_sha256,
                    report,
                })
            }
        }
    }

    fn persist_scheduled_repair(
        conn: &runlark_local::Connection,
        new_run: &Run,
        events: &[DomainEvent],
        reason_text: &str,
        is_team_managed: bool,
        events_out: &mut BufferedEvents,
        now: Timestamp,
    ) -> Result<(), PipelineError> {
        SessionStore::save_run(conn, new_run)?;
        if let Some(tid) = &new_run.task_id
            && let Some(mut task) = TaskStore::get_task(conn, tid)?
        {
            let expected = task.revision;
            task.workflow_stage = WorkflowStage::Implementation;
            if !is_team_managed {
                task.revision = Revision(expected.0 + 1);
            }
            task.updated_at = now;
            TaskStore::update_task_with_revision(conn, &task, expected)?;
        }
        for ev in events {
            let stored = EventStore::append_domain_event(conn, &new_run.id, ev, now)?;
            events_out.push((new_run.session_id.0.clone(), stored));
        }

        if let Some(mut session) = SessionStore::get_session(conn, &new_run.session_id)? {
            let expected_sess_rev = session.current_revision;
            let repair_prompt = SendPromptCommand {
                command_id: CommandId::from(format!(
                    "repair_{}_{}",
                    new_run.id.0, new_run.repair_attempts
                )),
                session_id: session.id.clone(),
                run_id: new_run.id.clone(),
                prompt: format!(
                    "Verification failed (repair attempt {}): {reason_text}. Fix and re-verify.",
                    new_run.repair_attempts
                ),
                expected_revision: expected_sess_rev,
                created_at: now,
            };
            session.current_revision = Revision(expected_sess_rev.0 + 1);
            session.updated_at = now;
            SessionStore::update_session_with_revision(conn, &session, expected_sess_rev)?;
            RunQueries::enqueue_outbox_prompt_in_tx(conn, &repair_prompt, now)?;
        }
        Ok(())
    }

    fn persist_halted_repair(
        conn: &runlark_local::Connection,
        new_run: &Run,
        block_reason: &BudgetBlockReason,
        events: &[DomainEvent],
        events_out: &mut BufferedEvents,
        now: Timestamp,
    ) -> Result<String, PipelineError> {
        SessionStore::save_run(conn, new_run)?;
        if let Some(tid) = &new_run.task_id {
            RunQueries::clear_task_active_run_if_matches(conn, tid, &new_run.id, now)?;
        }
        for ev in events {
            let stored = EventStore::append_domain_event(conn, &new_run.id, ev, now)?;
            events_out.push((new_run.session_id.0.clone(), stored));
        }
        let block_msg = block_reason.to_string();
        if let BudgetBlockReason::UnknownCostBlocked { unknown_attempts } = block_reason {
            let payload = serde_json::json!({
                "run_id": new_run.id.0,
                "unknown_attempts": unknown_attempts,
                "reason": block_msg,
            })
            .to_string();
            let stored = EventStore::append_wire_event(
                conn,
                &new_run.id,
                "UnknownCostBlocked",
                &payload,
                now,
            )?;
            events_out.push((new_run.session_id.0.clone(), stored));
        }
        Ok(block_msg)
    }
}

fn emit_artifact_attached(
    conn: &runlark_local::Connection,
    run_id: &RunId,
    artifact: &ArtifactRef,
    label: &str,
    session_id: &str,
    events_out: &mut BufferedEvents,
    now: Timestamp,
) -> Result<(), StorageError> {
    let ev = DomainEvent::ArtifactAttached {
        run_id: run_id.clone(),
        digest_sha256: artifact.digest_sha256.clone(),
        kind: artifact.kind,
        label: label.to_string(),
        timestamp: now,
    };
    let stored = EventStore::append_domain_event(conn, run_id, &ev, now)?;
    events_out.push((session_id.to_string(), stored));
    Ok(())
}

pub(super) fn run_project_check(worktree: &Path, command: &str) -> CheckCommandCapture {
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(worktree)
        .output();
    match output {
        Ok(out) => {
            let code = out.status.code().unwrap_or(-1);
            CheckCommandCapture {
                evidence: CheckEvidence {
                    name: command.to_string(),
                    passed: out.status.success(),
                    exit_code: code,
                },
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            }
        }
        Err(err) => CheckCommandCapture {
            evidence: CheckEvidence {
                name: command.to_string(),
                passed: false,
                exit_code: -1,
            },
            stdout: String::new(),
            stderr: format!("failed to execute project check `{command}`: {err}"),
        },
    }
}
