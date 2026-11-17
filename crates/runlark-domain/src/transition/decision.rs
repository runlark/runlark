use crate::decision::{Decision, DecisionBinding, DecisionKind, verify_decision_binding};
use crate::error::TransitionError;
use crate::event::DomainEvent;
use crate::id::{Revision, Timestamp};
use crate::state::{Run, RunStatus};
use crate::task::{Task, TaskStage, WorkflowStage};

/// Outcome of applying a bound human decision to a task and run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionOutcome {
    pub new_task: Task,
    pub new_run: Run,
    pub events: Vec<DomainEvent>,
}

fn validate_and_apply_kind(
    task: &Task,
    run: &Run,
    kind: DecisionKind,
    next_task: &mut Task,
    next_run: &mut Run,
    events: &mut Vec<DomainEvent>,
    now: Timestamp,
) -> Result<(), TransitionError> {
    match kind {
        DecisionKind::Accept => {
            if task.stage != TaskStage::Acceptance {
                return Err(TransitionError::InvalidTaskStageTransition {
                    from: task.stage,
                    to: TaskStage::Done,
                    reason: "task must be in Acceptance to record Accept decision".to_string(),
                });
            }
            if run.workflow_stage != WorkflowStage::HumanAcceptance {
                return Err(TransitionError::InvalidWorkflowStageTransition {
                    from: run.workflow_stage,
                    to: WorkflowStage::HumanAcceptance,
                });
            }
            next_task.stage = TaskStage::Done;
            next_task.active_run_id = None;
            next_run.status = RunStatus::Completed;
            events.push(DomainEvent::TaskStageChanged {
                task_id: task.id.clone(),
                from: TaskStage::Acceptance,
                to: TaskStage::Done,
                timestamp: now,
            });
            events.push(DomainEvent::RunCompleted {
                run_id: run.id.clone(),
                timestamp: now,
            });
        }
        DecisionKind::RequestChanges => {
            let from_stage = task.stage;
            let from_workflow = run.workflow_stage;
            next_task.stage = TaskStage::InProgress;
            next_task.workflow_stage = WorkflowStage::Implementation;
            next_run.workflow_stage = WorkflowStage::Implementation;
            if next_run.status == RunStatus::WaitingForApproval {
                next_run.status = RunStatus::Queued;
            }
            if from_stage != TaskStage::InProgress {
                events.push(DomainEvent::TaskStageChanged {
                    task_id: task.id.clone(),
                    from: from_stage,
                    to: TaskStage::InProgress,
                    timestamp: now,
                });
            }
            if from_workflow != WorkflowStage::Implementation {
                events.push(DomainEvent::RunStageAdvanced {
                    run_id: run.id.clone(),
                    from: from_workflow,
                    to: WorkflowStage::Implementation,
                    timestamp: now,
                });
            }
        }
        DecisionKind::ApprovePlan => {
            if run.workflow_stage != WorkflowStage::Plan {
                return Err(TransitionError::InvalidWorkflowStageTransition {
                    from: run.workflow_stage,
                    to: WorkflowStage::Implementation,
                });
            }
            next_task.workflow_stage = WorkflowStage::Implementation;
            next_run.workflow_stage = WorkflowStage::Implementation;
            if next_run.status == RunStatus::WaitingForApproval {
                next_run.status = RunStatus::Queued;
            }
            events.push(DomainEvent::RunStageAdvanced {
                run_id: run.id.clone(),
                from: WorkflowStage::Plan,
                to: WorkflowStage::Implementation,
                timestamp: now,
            });
        }
    }
    Ok(())
}

/// Applies a human [`Decision`] (`Accept`, `RequestChanges`, `ApprovePlan`) after verifying
/// binding to `plan_version`, `head_sha`, `base_sha`, `policy_digest`, and `evidence_digest`.
///
/// # Errors
/// Returns [`TransitionError`] if the task/run IDs mismatch, any binding drifted, or the task/run
/// is not in the required stage (`Acceptance`/`HumanAcceptance` for `Accept`, `Plan` for `ApprovePlan`).
pub fn apply_decision(
    task: &Task,
    run: &Run,
    decision: &Decision,
    binding: &DecisionBinding<'_>,
    now: Timestamp,
) -> Result<DecisionOutcome, TransitionError> {
    if decision.task_id != task.id {
        return Err(TransitionError::InvalidRunState(format!(
            "decision task {} does not match {}",
            decision.task_id, task.id
        )));
    }
    if decision.run_id != run.id {
        return Err(TransitionError::ActiveRunMismatch {
            active: run.id.clone(),
            requested: decision.run_id.clone(),
        });
    }

    verify_decision_binding(decision, binding)?;

    let mut next_task = task.clone();
    let mut next_run = run.clone();
    let mut events = vec![DomainEvent::DecisionRecorded {
        decision_id: decision.id.clone(),
        task_id: task.id.clone(),
        run_id: run.id.clone(),
        kind: decision.kind,
        actor: decision.actor.clone(),
        plan_version: decision.plan_version,
        head_sha: decision.head_sha.clone(),
        evidence_digest: decision.evidence_digest.clone(),
        timestamp: now,
    }];

    validate_and_apply_kind(
        task,
        run,
        decision.kind,
        &mut next_task,
        &mut next_run,
        &mut events,
        now,
    )?;

    next_task.revision = Revision(task.revision.0 + 1);
    next_task.updated_at = now;
    next_run.revision = Revision(run.revision.0 + 1);
    next_run.updated_at = now;

    Ok(DecisionOutcome {
        new_task: next_task,
        new_run: next_run,
        events,
    })
}
