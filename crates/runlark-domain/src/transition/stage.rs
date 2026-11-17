use crate::budget::{Attempt, BudgetBlockReason, BudgetPolicy, CostSummary, can_retry_repair};
use crate::error::TransitionError;
use crate::event::DomainEvent;
use crate::id::{Revision, Timestamp};
use crate::state::{Run, RunStatus};
use crate::task::WorkflowStage;

/// Outcome of advancing a run's internal workflow stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageTransitionOutcome {
    pub new_run: Run,
    pub events: Vec<DomainEvent>,
}

/// Outcome of evaluating a bounded repair loop retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairScheduleOutcome {
    Scheduled {
        new_run: Run,
        events: Vec<DomainEvent>,
    },
    Halted {
        new_run: Run,
        reason: BudgetBlockReason,
        events: Vec<DomainEvent>,
    },
}

/// Advances a run along its workflow pipeline (`Clarification -> Plan -> Implementation ->
/// Verification -> HumanAcceptance`), enforcing `QuickFix` vs `Standard` stage rules.
///
/// Advancing into `HumanAcceptance` moves `RunStatus` to `WaitingForApproval` so the execution
/// slot is freed while awaiting human acceptance.
///
/// # Errors
/// Returns [`TransitionError`] if the run is terminal/cancelling or the stage edge is invalid.
pub fn advance_run_stage(
    run: &Run,
    target_stage: WorkflowStage,
    now: Timestamp,
) -> Result<StageTransitionOutcome, TransitionError> {
    if run.status.is_terminal() {
        return Err(TransitionError::RunAlreadyTerminated);
    }
    if run.status == RunStatus::CancelRequested {
        return Err(TransitionError::RunCancelling);
    }
    if run.workflow_stage == target_stage {
        return Ok(StageTransitionOutcome {
            new_run: run.clone(),
            events: Vec::new(),
        });
    }
    if !run
        .workflow_stage
        .can_advance_to(target_stage, run.task_kind)
    {
        return Err(TransitionError::InvalidWorkflowStageTransition {
            from: run.workflow_stage,
            to: target_stage,
        });
    }

    let from = run.workflow_stage;
    let mut next_run = run.clone();
    next_run.workflow_stage = target_stage;
    if target_stage == WorkflowStage::HumanAcceptance {
        next_run.status = RunStatus::WaitingForApproval;
    }
    next_run.revision = Revision(run.revision.0 + 1);
    next_run.updated_at = now;

    Ok(StageTransitionOutcome {
        new_run: next_run,
        events: vec![DomainEvent::RunStageAdvanced {
            run_id: run.id.clone(),
            from,
            to: target_stage,
            timestamp: now,
        }],
    })
}

/// Records an [`Attempt`] into the run's explicit [`CostSummary`] ledger.
///
/// Unknown cost (`attempt.cost_micro_usd == None`) increments `unknown_attempts` and emits
/// [`DomainEvent::UnknownCostRecorded`] without ever folding into `0`.
#[must_use]
pub fn record_attempt_and_budget(
    run: &Run,
    prior_attempts: &[Attempt],
    attempt: &Attempt,
    now: Timestamp,
) -> StageTransitionOutcome {
    let mut all_attempts = Vec::with_capacity(prior_attempts.len() + 1);
    all_attempts.extend_from_slice(prior_attempts);
    all_attempts.push(attempt.clone());

    let mut next_run = run.clone();
    next_run.cost_summary =
        CostSummary::from_attempts(&all_attempts, run.cost_summary.held_micro_usd);
    next_run.revision = Revision(run.revision.0 + 1);
    next_run.updated_at = now;

    let mut events = vec![DomainEvent::AttemptRecorded {
        attempt_id: attempt.id.clone(),
        run_id: run.id.clone(),
        attempt_no: attempt.attempt_no,
        workflow_stage: attempt.workflow_stage,
        cost_micro_usd: attempt.cost_micro_usd,
        timestamp: now,
    }];

    if attempt.cost_micro_usd.is_none() {
        events.push(DomainEvent::UnknownCostRecorded {
            run_id: run.id.clone(),
            attempt_id: attempt.id.clone(),
            reason: attempt
                .unknown_cost_reason
                .clone()
                .unwrap_or_else(|| "provider did not report attempt cost".to_string()),
            timestamp: now,
        });
    }

    StageTransitionOutcome {
        new_run: next_run,
        events,
    }
}

/// Checks bounded repair eligibility and either schedules a repair (`Verification ->
/// Implementation`) or halts the run (`RunStatus::Failed`) when budget/unknown-cost limits block.
///
/// # Errors
/// Returns [`TransitionError::InvalidRunState`] if the run is not in `Verification` or is already
/// terminal.
pub fn schedule_repair(
    run: &Run,
    attempts: &[Attempt],
    policy: &BudgetPolicy,
    elapsed_ms: u64,
    repair_reason: impl Into<String>,
    now: Timestamp,
) -> Result<RepairScheduleOutcome, TransitionError> {
    if run.status.is_terminal() {
        return Err(TransitionError::RunAlreadyTerminated);
    }
    if run.workflow_stage != WorkflowStage::Verification {
        return Err(TransitionError::InvalidRunState(format!(
            "cannot schedule repair from stage {:?}",
            run.workflow_stage
        )));
    }

    let reason_text = repair_reason.into();
    match can_retry_repair(run, attempts, policy, elapsed_ms) {
        Ok(()) => {
            let mut next_run = run.clone();
            next_run.workflow_stage = WorkflowStage::Implementation;
            next_run.repair_attempts = run.repair_attempts.saturating_add(1);
            next_run.revision = Revision(run.revision.0 + 1);
            next_run.updated_at = now;

            let events = vec![
                DomainEvent::RunStageAdvanced {
                    run_id: run.id.clone(),
                    from: WorkflowStage::Verification,
                    to: WorkflowStage::Implementation,
                    timestamp: now,
                },
                DomainEvent::RepairScheduled {
                    run_id: run.id.clone(),
                    repair_attempt: next_run.repair_attempts,
                    reason: reason_text,
                    timestamp: now,
                },
            ];

            Ok(RepairScheduleOutcome::Scheduled {
                new_run: next_run,
                events,
            })
        }
        Err(block_reason) => {
            let msg = block_reason.to_string();
            let mut next_run = run.clone();
            next_run.status = RunStatus::Failed;
            next_run.revision = Revision(run.revision.0 + 1);
            next_run.updated_at = now;

            let events = vec![
                DomainEvent::BudgetExceeded {
                    run_id: run.id.clone(),
                    reason: msg.clone(),
                    timestamp: now,
                },
                DomainEvent::RunFailed {
                    run_id: run.id.clone(),
                    error: msg,
                    timestamp: now,
                },
            ];

            Ok(RepairScheduleOutcome::Halted {
                new_run: next_run,
                reason: block_reason,
                events,
            })
        }
    }
}
