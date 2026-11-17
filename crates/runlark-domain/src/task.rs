use crate::budget::BudgetPolicy;
use crate::error::TransitionError;
use crate::id::{ProjectId, Revision, RunId, SessionId, TaskId, Timestamp};
use serde::{Deserialize, Serialize};

/// Classification of a task determining whether planning/clarification stages are required.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    QuickFix,
    #[default]
    Standard,
}

impl TaskKind {
    /// Returns the initial workflow stage for this task kind.
    ///
    /// `QuickFix` tasks skip `Clarification` and `Plan`, starting directly at `Implementation`.
    #[must_use]
    pub const fn initial_workflow_stage(self) -> WorkflowStage {
        match self {
            Self::QuickFix => WorkflowStage::Implementation,
            Self::Standard => WorkflowStage::Clarification,
        }
    }
}

/// Kanban board column / lifecycle stage for a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskStage {
    #[default]
    Inbox,
    Ready,
    InProgress,
    Acceptance,
    Done,
}

/// Internal workflow pipeline stage inside a run execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStage {
    Clarification,
    Plan,
    #[default]
    Implementation,
    Verification,
    HumanAcceptance,
}

impl WorkflowStage {
    /// Returns the canonical next stage in the workflow pipeline for the given `TaskKind`.
    #[must_use]
    pub const fn next(self, kind: TaskKind) -> Option<Self> {
        match (self, kind) {
            (Self::Clarification, TaskKind::Standard) => Some(Self::Plan),
            (Self::Clarification, TaskKind::QuickFix) | (Self::Plan, _) => {
                Some(Self::Implementation)
            }
            (Self::Implementation, _) => Some(Self::Verification),
            (Self::Verification, _) => Some(Self::HumanAcceptance),
            (Self::HumanAcceptance, _) => None,
        }
    }

    /// Returns true if transitioning from `self` to `target` is valid for `kind`.
    ///
    /// Allows forward progression (`Clarification -> Plan -> Implementation -> Verification ->
    /// HumanAcceptance`), `QuickFix` skipping `Clarification`/`Plan` into `Implementation`,
    /// bounded repair (`Verification -> Implementation`), and change requests
    /// (`HumanAcceptance -> Implementation`).
    #[must_use]
    pub const fn can_advance_to(self, target: Self, kind: TaskKind) -> bool {
        matches!(
            (self, target, kind),
            (Self::Clarification, Self::Plan, TaskKind::Standard)
                | (
                    Self::Clarification,
                    Self::Implementation,
                    TaskKind::QuickFix
                )
                | (
                    Self::Plan,
                    Self::Implementation,
                    TaskKind::Standard | TaskKind::QuickFix
                )
                | (Self::Implementation, Self::Verification, _)
                | (
                    Self::Verification,
                    Self::HumanAcceptance | Self::Implementation,
                    _
                )
                | (Self::HumanAcceptance, Self::Implementation, _)
        )
    }
}

/// Actor initiating a board or task transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", content = "actor", rename_all = "snake_case")]
pub enum ActorRole {
    Human(String),
    Agent(String),
    System,
}

impl ActorRole {
    /// Returns `true` if this actor is a non-empty human reviewer.
    #[must_use]
    pub fn is_human(&self) -> bool {
        matches!(self, Self::Human(name) if !name.trim().is_empty())
    }
}

/// Durable Kanban task state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub project_id: ProjectId,
    pub title: String,
    pub description: String,
    pub kind: TaskKind,
    pub stage: TaskStage,
    pub workflow_stage: WorkflowStage,
    pub plan_text: Option<String>,
    pub plan_version: u32,
    pub base_ref: Option<String>,
    pub head_ref: Option<String>,
    pub pr_url: Option<String>,
    pub active_session_id: Option<SessionId>,
    pub active_run_id: Option<RunId>,
    pub latest_run_id: Option<RunId>,
    pub budget_override: Option<BudgetPolicy>,
    pub revision: Revision,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Validates a board stage move on `task`, enforcing human-only acceptance and active-run guards.
///
/// # Errors
/// Returns [`TransitionError`] if an agent tries to move a task to `Done` / self-accept,
/// if moving to `Done` lacks a human acceptance decision, if an active executing run blocks
/// moving backwards out of `InProgress`, or if the column transition is not allowed.
pub fn validate_task_stage_move(
    task: &Task,
    target: TaskStage,
    actor: &ActorRole,
    has_executing_run: bool,
    has_accept_decision: bool,
) -> Result<(), TransitionError> {
    if matches!(actor, ActorRole::Agent(_))
        && matches!(target, TaskStage::Done | TaskStage::Acceptance)
        && (target == TaskStage::Done || task.stage == TaskStage::Acceptance)
    {
        return Err(TransitionError::AgentCannotSelfAccept);
    }

    if task.stage == target {
        return Ok(());
    }

    if target == TaskStage::Done {
        if !actor.is_human() {
            return Err(TransitionError::AgentCannotSelfAccept);
        }
        if task.stage != TaskStage::Acceptance {
            return Err(TransitionError::InvalidTaskStageTransition {
                from: task.stage,
                to: target,
                reason: "task must be in Acceptance before moving to Done".to_string(),
            });
        }
        if !has_accept_decision {
            return Err(TransitionError::AcceptanceDecisionRequired);
        }
        return Ok(());
    }

    if task.stage == TaskStage::InProgress
        && matches!(target, TaskStage::Inbox | TaskStage::Ready)
        && has_executing_run
    {
        if let Some(run_id) = &task.active_run_id {
            return Err(TransitionError::TaskHasActiveRun {
                run_id: run_id.clone(),
            });
        }
        return Err(TransitionError::InvalidTaskStageTransition {
            from: task.stage,
            to: target,
            reason: "cannot move task back while run is executing".to_string(),
        });
    }

    let allowed = matches!(
        (task.stage, target),
        (TaskStage::Inbox, TaskStage::Ready)
            | (TaskStage::Ready, TaskStage::Inbox | TaskStage::InProgress)
            | (
                TaskStage::InProgress,
                TaskStage::Ready | TaskStage::Acceptance
            )
            | (
                TaskStage::Acceptance,
                TaskStage::InProgress | TaskStage::Ready
            )
    );

    if !allowed {
        return Err(TransitionError::InvalidTaskStageTransition {
            from: task.stage,
            to: target,
            reason: "unsupported board column transition".to_string(),
        });
    }

    Ok(())
}
