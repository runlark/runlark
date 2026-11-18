use super::{PipelineError, VerifyPipelineOutcome};
use crate::runner_agent::{RunnerAgentWorker, RunnerStateInput};
use crate::state::AppState;
use crate::team_http::TeamServerError;
use crate::team_sync::TeamSyncWorker;
use runlark_domain::{
    ActorRole, ArtifactKind, AttachArtifactCommand, CommandId, LeaseEpoch, MoveTaskStageCommand,
    ProjectOwnershipMode, Revision, Run, RunId, RunStatus, RunnerId, Task, TaskId, TaskStage,
    TeamCommand, Timestamp, WorkflowStage, WorkspaceId,
};
use runlark_local::{
    ArtifactCasStore, Connection, SyncOutboxStore, SyncStore, SyncedCasStore, TeamAuthStore,
    open_sqlite, sha256_hex,
};
use serde_json::json;

/// Verified local runner lease context for a `TeamManaged` run.
#[derive(Debug, Clone)]
pub(super) struct TeamRunLease {
    pub(super) workspace_id: WorkspaceId,
    pub(super) runner_id: RunnerId,
    pub(super) lease_epoch: LeaseEpoch,
    pub(super) server_url: String,
    pub(super) bearer_token: String,
}

/// Snapshot of task revision/stage captured before local pipeline transition.
#[derive(Debug, Clone)]
pub(super) struct TaskBeforeMeta {
    pub(super) id: TaskId,
    pub(super) stage: TaskStage,
    pub(super) revision: Revision,
}

impl From<&Task> for TaskBeforeMeta {
    fn from(task: &Task) -> Self {
        Self {
            id: task.id.clone(),
            stage: task.stage,
            revision: task.revision,
        }
    }
}

/// Resolves and verifies this daemon's runner lease if `run` belongs to a `TeamManaged` project.
///
/// Returns `Ok(None)` for `SoloLocal` projects, or `Err(PipelineError::TeamFencing)` if a
/// `TeamManaged` run lacks an active matching lease on this daemon.
pub(super) fn resolve_team_run_lease(
    conn: &Connection,
    run: &Run,
    task_opt: Option<&Task>,
) -> Result<Option<TeamRunLease>, PipelineError> {
    let project_id = run
        .project_id
        .as_ref()
        .or_else(|| task_opt.map(|t| &t.project_id));
    let Some(pid) = project_id else {
        return Ok(None);
    };
    let ProjectOwnershipMode::TeamManaged { workspace_id, .. } =
        SyncStore::get_ownership_mode(conn, pid)?
    else {
        return Ok(None);
    };
    let ownership = SyncStore::get_run_ownership(conn, &run.id)?.unwrap_or_default();
    if ownership.runner_unreachable {
        return Err(PipelineError::TeamFencing(format!(
            "run {} owner is marked unreachable",
            run.id
        )));
    }
    let Some(assigned_runner) = ownership.assigned_runner_id else {
        return Err(PipelineError::TeamFencing(format!(
            "team run {} has no assigned runner lease",
            run.id
        )));
    };
    if ownership.lease_epoch.0 == 0 {
        return Err(PipelineError::TeamFencing(format!(
            "team run {} has inactive lease epoch 0",
            run.id
        )));
    }
    let auth =
        TeamAuthStore::resolve_for_workspace(conn, Some(&workspace_id))?.ok_or_else(|| {
            PipelineError::TeamFencing(format!(
                "no team auth session configured for workspace {workspace_id}"
            ))
        })?;
    if auth.runner_id.as_ref() != Some(&assigned_runner) {
        return Err(PipelineError::TeamFencing(format!(
            "run {} is leased to runner {}, not {:?}",
            run.id, assigned_runner, auth.runner_id
        )));
    }
    Ok(Some(TeamRunLease {
        workspace_id,
        runner_id: assigned_runner,
        lease_epoch: ownership.lease_epoch,
        server_url: auth.server_url,
        bearer_token: auth.bearer_token,
    }))
}

/// Reports (or offline-buffers) fenced stage progression for `WorkflowPipeline::advance_stage_with_task`.
pub(super) async fn report_stage_advance(
    state: &AppState,
    lease: &TeamRunLease,
    new_run: &Run,
    task_before: Option<&TaskBeforeMeta>,
    target_stage: WorkflowStage,
    now: Timestamp,
) -> Result<(), PipelineError> {
    let payload = json!({
        "workflow_stage": target_stage,
        "status": new_run.status,
    })
    .to_string();
    let status_patch =
        (target_stage == WorkflowStage::HumanAcceptance).then_some(RunStatus::WaitingForApproval);
    report_fenced_step(
        state,
        lease,
        FencedStep {
            run_id: &new_run.id,
            status: status_patch,
            workflow_stage: Some(target_stage),
            head_sha: new_run.head_sha.as_deref(),
            base_sha: new_run.base_sha.as_deref(),
            event_type: "run_stage_advanced",
            payload_json: &payload,
        },
    )
    .await?;
    if target_stage == WorkflowStage::HumanAcceptance
        && let Some(tb) = task_before
        && tb.stage != TaskStage::Acceptance
    {
        sync_or_queue_acceptance_move(state, lease, &new_run.id, tb, now).await?;
    }
    Ok(())
}

/// Inputs for reporting or buffering verification pipeline results for a `TeamManaged` run.
pub(super) struct VerificationReportInput<'a> {
    pub(super) run_id: &'a RunId,
    pub(super) started_at_implementation: bool,
    pub(super) head_sha: &'a str,
    pub(super) base_sha: &'a str,
    pub(super) task_before: Option<&'a TaskBeforeMeta>,
    pub(super) outcome: &'a VerifyPipelineOutcome,
    pub(super) now: Timestamp,
}

/// Reports (or offline-buffers) fenced verification transitions and task stage move for
/// `WorkflowPipeline::verify_and_advance`.
pub(super) async fn report_verification_outcome(
    state: &AppState,
    lease: &TeamRunLease,
    input: &VerificationReportInput<'_>,
) -> Result<(), PipelineError> {
    if input.started_at_implementation {
        let verif_payload = json!({
            "workflow_stage": WorkflowStage::Verification,
            "head_sha": input.head_sha,
            "base_sha": input.base_sha,
        })
        .to_string();
        report_fenced_step(
            state,
            lease,
            FencedStep {
                run_id: input.run_id,
                status: None,
                workflow_stage: Some(WorkflowStage::Verification),
                head_sha: Some(input.head_sha),
                base_sha: Some(input.base_sha),
                event_type: "run_stage_verification",
                payload_json: &verif_payload,
            },
        )
        .await?;
    }

    match input.outcome {
        VerifyPipelineOutcome::PassedForAcceptance {
            evidence_digest, ..
        } => report_passed_acceptance(state, lease, input, evidence_digest.as_str()).await,
        VerifyPipelineOutcome::RepairScheduled {
            repair_attempt,
            reason,
            ..
        } => {
            let payload = json!({
                "workflow_stage": WorkflowStage::Implementation,
                "repair_attempt": repair_attempt,
                "reason": reason,
            })
            .to_string();
            report_fenced_step(
                state,
                lease,
                FencedStep {
                    run_id: input.run_id,
                    status: None,
                    workflow_stage: Some(WorkflowStage::Implementation),
                    head_sha: Some(input.head_sha),
                    base_sha: Some(input.base_sha),
                    event_type: "run_repair_scheduled",
                    payload_json: &payload,
                },
            )
            .await?;
            Ok(())
        }
        VerifyPipelineOutcome::BudgetHalted {
            block_reason,
            reason,
            ..
        } => {
            let payload = json!({
                "status": RunStatus::Failed,
                "block_reason": block_reason,
                "reason": reason,
            })
            .to_string();
            report_fenced_step(
                state,
                lease,
                FencedStep {
                    run_id: input.run_id,
                    status: Some(RunStatus::Failed),
                    workflow_stage: None,
                    head_sha: Some(input.head_sha),
                    base_sha: Some(input.base_sha),
                    event_type: "run_budget_halted",
                    payload_json: &payload,
                },
            )
            .await?;
            Ok(())
        }
    }
}

async fn report_passed_acceptance(
    state: &AppState,
    lease: &TeamRunLease,
    input: &VerificationReportInput<'_>,
    evidence_digest: &str,
) -> Result<(), PipelineError> {
    let payload = json!({
        "workflow_stage": WorkflowStage::HumanAcceptance,
        "status": RunStatus::WaitingForApproval,
        "head_sha": input.head_sha,
        "base_sha": input.base_sha,
        "evidence_digest": evidence_digest,
    })
    .to_string();
    report_fenced_step(
        state,
        lease,
        FencedStep {
            run_id: input.run_id,
            status: Some(RunStatus::WaitingForApproval),
            workflow_stage: Some(WorkflowStage::HumanAcceptance),
            head_sha: Some(input.head_sha),
            base_sha: Some(input.base_sha),
            event_type: "run_passed_for_acceptance",
            payload_json: &payload,
        },
    )
    .await?;
    sync_or_queue_evidence_artifacts(state, lease, input.run_id, input.now).await?;
    if let Some(tb) = input.task_before
        && tb.stage != TaskStage::Acceptance
    {
        sync_or_queue_acceptance_move(state, lease, input.run_id, tb, input.now).await?;
    }
    Ok(())
}

struct FencedStep<'a> {
    run_id: &'a RunId,
    status: Option<RunStatus>,
    workflow_stage: Option<WorkflowStage>,
    head_sha: Option<&'a str>,
    base_sha: Option<&'a str>,
    event_type: &'a str,
    payload_json: &'a str,
}

async fn report_fenced_step(
    state: &AppState,
    lease: &TeamRunLease,
    step: FencedStep<'_>,
) -> Result<bool, PipelineError> {
    let worker = RunnerAgentWorker::new(state.clone());
    worker
        .report_or_buffer_run_state(RunnerStateInput {
            workspace_id: &lease.workspace_id,
            run_id: step.run_id,
            runner_id: &lease.runner_id,
            lease_epoch: lease.lease_epoch,
            status: step.status,
            workflow_stage: step.workflow_stage,
            head_sha: step.head_sha.map(str::to_string),
            base_sha: step.base_sha.map(str::to_string),
            event_type: step.event_type,
            payload_json: step.payload_json,
        })
        .await
        .map_err(|err| match err {
            TeamServerError::Storage(s) => PipelineError::Storage(s),
            other => PipelineError::TeamFencing(other.to_string()),
        })
}

async fn sync_or_queue_evidence_artifacts(
    state: &AppState,
    lease: &TeamRunLease,
    run_id: &RunId,
    now: Timestamp,
) -> Result<(), PipelineError> {
    let linked = {
        let conn = open_sqlite(&state.db_path)?;
        ArtifactCasStore::list_artifacts_for_run(&conn, run_id)?
    };
    let cas = state.cas_store();
    let sync = TeamSyncWorker::new(state.clone());

    for (meta, label) in linked {
        if meta.kind != ArtifactKind::EvidenceBundle || !cas.has_blob(&meta.digest_sha256) {
            continue;
        }
        let command_id =
            TeamSyncWorker::deterministic_attach_command_id(run_id, &meta.digest_sha256, &label);
        let digest = meta.digest_sha256.clone();
        let attach_cmd = TeamCommand::AttachArtifact(AttachArtifactCommand {
            command_id,
            run_id: run_id.clone(),
            digest_sha256: meta.digest_sha256.clone(),
            kind: meta.kind,
            media_type: meta.media_type.clone(),
            size_bytes: meta.size_bytes,
            label,
            created_at: now,
        });
        let bytes = cas.get_bytes(&digest)?;
        match sync
            .upload_artifact(
                &lease.server_url,
                &lease.bearer_token,
                &lease.workspace_id,
                &meta,
                &bytes,
            )
            .await
        {
            Ok(_) => {
                let conn = open_sqlite(&state.db_path)?;
                SyncedCasStore::mark_uploaded(&conn, &lease.workspace_id, &digest, now)?;
                drop(conn);
                submit_or_queue_team_command(state, lease, &attach_cmd, None, now).await?;
            }
            Err(TeamServerError::Offline(_)) => {
                queue_outbox_command(state, &lease.workspace_id, &attach_cmd, None, now)?;
            }
            Err(err) => return Err(map_team_error(err)),
        }
    }
    Ok(())
}

async fn sync_or_queue_acceptance_move(
    state: &AppState,
    lease: &TeamRunLease,
    run_id: &RunId,
    task_before: &TaskBeforeMeta,
    now: Timestamp,
) -> Result<(), PipelineError> {
    let hash = sha256_hex(
        format!(
            "{}\0{}\0{}",
            run_id.as_ref(),
            task_before.id.as_ref(),
            task_before.revision.0
        )
        .as_bytes(),
    );
    let command_id = CommandId::from(format!("cmd_mv_acc_{}", &hash[..20]));
    let cmd = TeamCommand::MoveTaskStage(MoveTaskStageCommand {
        command_id,
        task_id: task_before.id.clone(),
        target_stage: TaskStage::Acceptance,
        actor: ActorRole::Agent(lease.runner_id.0.clone()),
        expected_revision: task_before.revision,
        created_at: now,
    });
    submit_or_queue_team_command(state, lease, &cmd, Some(task_before.revision), now).await
}

async fn submit_or_queue_team_command(
    state: &AppState,
    lease: &TeamRunLease,
    cmd: &TeamCommand,
    expected_revision: Option<Revision>,
    now: Timestamp,
) -> Result<(), PipelineError> {
    let sync = TeamSyncWorker::new(state.clone());
    match sync
        .submit_command_and_sync(
            &lease.server_url,
            &lease.bearer_token,
            &lease.workspace_id,
            cmd,
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(TeamServerError::Offline(_)) => {
            queue_outbox_command(state, &lease.workspace_id, cmd, expected_revision, now)
        }
        Err(err) => Err(map_team_error(err)),
    }
}

fn queue_outbox_command(
    state: &AppState,
    workspace_id: &WorkspaceId,
    cmd: &TeamCommand,
    expected_revision: Option<Revision>,
    now: Timestamp,
) -> Result<(), PipelineError> {
    let conn = open_sqlite(&state.db_path)?;
    if SyncOutboxStore::get(&conn, cmd.command_id())?.is_none() {
        let entry = TeamSyncWorker::build_outbox_entry(
            workspace_id,
            None,
            None,
            cmd,
            expected_revision,
            now,
        )
        .map_err(map_team_error)?;
        SyncOutboxStore::enqueue(&conn, &entry)?;
    }
    Ok(())
}

fn map_team_error(err: TeamServerError) -> PipelineError {
    match err {
        TeamServerError::Storage(s) => PipelineError::Storage(s),
        other => PipelineError::TeamFencing(other.to_string()),
    }
}
