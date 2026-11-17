use crate::error::TransitionError;
use crate::event::DomainEvent;
use crate::id::{Revision, RunId, SessionId, Timestamp};
use crate::task::{ActorRole, Task, TaskKind, TaskStage, WorkflowStage, validate_task_stage_move};

/// Result of a pure task board transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskTransitionOutcome {
    pub new_task: Task,
    pub events: Vec<DomainEvent>,
}

/// Moves a task between Kanban columns, enforcing stage rules, active-run protection,
/// and human-only acceptance invariants.
///
/// # Errors
/// Returns [`TransitionError`] on revision mismatch, invalid column transition, agent
/// self-acceptance attempt, missing acceptance decision, or active run conflict.
pub fn move_task_stage(
    task: &Task,
    target_stage: TaskStage,
    actor: &ActorRole,
    expected_revision: Revision,
    has_executing_run: bool,
    has_accept_decision: bool,
    now: Timestamp,
) -> Result<TaskTransitionOutcome, TransitionError> {
    if task.revision != expected_revision {
        return Err(TransitionError::RevisionMismatch {
            expected: expected_revision,
            actual: task.revision,
        });
    }

    validate_task_stage_move(
        task,
        target_stage,
        actor,
        has_executing_run,
        has_accept_decision,
    )?;

    let mut next = task.clone();
    if task.stage != target_stage {
        next.stage = target_stage;
        next.revision = Revision(task.revision.0 + 1);
        next.updated_at = now;
        return Ok(TaskTransitionOutcome {
            new_task: next,
            events: vec![DomainEvent::TaskStageChanged {
                task_id: task.id.clone(),
                from: task.stage,
                to: target_stage,
                timestamp: now,
            }],
        });
    }

    Ok(TaskTransitionOutcome {
        new_task: next,
        events: Vec::new(),
    })
}

/// Updates a task's plan text and increments `plan_version`.
///
/// # Errors
/// Returns [`TransitionError::RevisionMismatch`] if `expected_revision` does not match.
pub fn update_task_plan(
    task: &Task,
    plan_text: impl Into<String>,
    expected_revision: Revision,
    now: Timestamp,
) -> Result<TaskTransitionOutcome, TransitionError> {
    if task.revision != expected_revision {
        return Err(TransitionError::RevisionMismatch {
            expected: expected_revision,
            actual: task.revision,
        });
    }

    let mut next = task.clone();
    next.plan_text = Some(plan_text.into());
    next.plan_version = task.plan_version.saturating_add(1);
    next.revision = Revision(task.revision.0 + 1);
    next.updated_at = now;

    Ok(TaskTransitionOutcome {
        new_task: next.clone(),
        events: vec![DomainEvent::TaskPlanUpdated {
            task_id: next.id,
            plan_version: next.plan_version,
            timestamp: now,
        }],
    })
}

/// Attaches a new run/session to a task and transitions `Inbox`/`Ready` -> `InProgress`.
///
/// # Errors
/// Returns [`TransitionError`] on revision mismatch, existing active run, or if the task is
/// already `Done`.
pub fn start_task_run(
    task: &Task,
    session_id: &SessionId,
    run_id: &RunId,
    has_executing_run: bool,
    expected_revision: Revision,
    now: Timestamp,
) -> Result<TaskTransitionOutcome, TransitionError> {
    if task.revision != expected_revision {
        return Err(TransitionError::RevisionMismatch {
            expected: expected_revision,
            actual: task.revision,
        });
    }
    if has_executing_run {
        if let Some(active) = &task.active_run_id {
            return Err(TransitionError::TaskHasActiveRun {
                run_id: active.clone(),
            });
        }
        return Err(TransitionError::InvalidTaskStageTransition {
            from: task.stage,
            to: TaskStage::InProgress,
            reason: "task already has an active run".to_string(),
        });
    }
    if task.stage == TaskStage::Done {
        return Err(TransitionError::InvalidTaskStageTransition {
            from: task.stage,
            to: TaskStage::InProgress,
            reason: "cannot start a run for a completed Done task".to_string(),
        });
    }

    let from_stage = task.stage;
    let mut next = task.clone();
    next.stage = TaskStage::InProgress;
    next.workflow_stage = match task.kind {
        TaskKind::QuickFix => WorkflowStage::Implementation,
        TaskKind::Standard => match task.workflow_stage {
            WorkflowStage::Clarification | WorkflowStage::Plan | WorkflowStage::Implementation => {
                task.workflow_stage
            }
            WorkflowStage::Verification | WorkflowStage::HumanAcceptance => {
                WorkflowStage::Implementation
            }
        },
    };
    next.active_session_id = Some(session_id.clone());
    next.active_run_id = Some(run_id.clone());
    next.latest_run_id = Some(run_id.clone());
    next.revision = Revision(task.revision.0 + 1);
    next.updated_at = now;

    let mut events = Vec::new();
    if from_stage != TaskStage::InProgress {
        events.push(DomainEvent::TaskStageChanged {
            task_id: task.id.clone(),
            from: from_stage,
            to: TaskStage::InProgress,
            timestamp: now,
        });
    }

    Ok(TaskTransitionOutcome {
        new_task: next,
        events,
    })
}
