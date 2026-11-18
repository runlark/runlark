use crate::id::{LeaseEpoch, Revision, RunId, RunnerId, SessionId};
use crate::task::{TaskStage, WorkflowStage};
use thiserror::Error;

/// Errors arising from invalid state transitions.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransitionError {
    #[error("Session is already archived")]
    SessionAlreadyArchived,

    #[error("Session mismatch: expected {expected}, got {actual}")]
    SessionMismatch {
        expected: SessionId,
        actual: SessionId,
    },

    #[error("Revision mismatch: expected {expected}, actual {actual}")]
    RevisionMismatch {
        expected: Revision,
        actual: Revision,
    },

    #[error("No active run found for session")]
    NoActiveRun,

    #[error("Run has already terminated")]
    RunAlreadyTerminated,

    #[error("Run is currently cancelling")]
    RunCancelling,

    #[error("Run is waiting for human approval or question answer")]
    RunWaitingForApproval,

    #[error("Run is not waiting for human approval or question answer")]
    RunNotWaitingForApproval,

    #[error("Active run mismatch: active {active}, requested {requested}")]
    ActiveRunMismatch { active: RunId, requested: RunId },

    #[error("Cannot change working directory while a run is active")]
    CwdConflictActiveRun,

    #[error("Cannot archive session while a run is active")]
    ArchiveBlockedByActiveRun,

    #[error("Invalid task stage transition from {from:?} to {to:?}: {reason}")]
    InvalidTaskStageTransition {
        from: TaskStage,
        to: TaskStage,
        reason: String,
    },

    #[error("Invalid workflow stage transition from {from:?} to {to:?}")]
    InvalidWorkflowStageTransition {
        from: WorkflowStage,
        to: WorkflowStage,
    },

    #[error("Agents cannot self-accept tasks or move tasks to Done")]
    AgentCannotSelfAccept,

    #[error("Human Decision::Accept is required before moving a task to Done")]
    AcceptanceDecisionRequired,

    #[error("Task has an active executing run ({run_id})")]
    TaskHasActiveRun { run_id: RunId },

    #[error("Missing decision binding field: {field}")]
    MissingDecisionBinding { field: String },

    #[error("Decision binding mismatch for {field}: expected {expected}, actual {actual}")]
    DecisionBindingMismatch {
        field: String,
        expected: String,
        actual: String,
    },

    #[error("Budget exceeded: {reason}")]
    BudgetExceeded { reason: String },

    #[error("Repair blocked by {unknown_attempts} unpriced attempt(s) with unknown cost")]
    UnknownCostBlocked { unknown_attempts: u32 },

    #[error("Invalid run state: {0}")]
    InvalidRunState(String),

    #[error("Workspace permission denied for action '{action}' with role '{role}'")]
    WorkspacePermissionDenied { action: String, role: String },

    #[error(
        "Runner fencing mismatch on run {run_id}: expected ({expected_runner:?}, epoch {expected_epoch}), got ({actual_runner}, epoch {actual_epoch})"
    )]
    RunnerFencingMismatch {
        run_id: RunId,
        expected_runner: Option<RunnerId>,
        expected_epoch: LeaseEpoch,
        actual_runner: RunnerId,
        actual_epoch: LeaseEpoch,
    },

    #[error(
        "Active run {run_id} is owned by runner {owner_runner_id}; automatic takeover is forbidden"
    )]
    ActiveRunPreventsTakeover {
        run_id: RunId,
        owner_runner_id: RunnerId,
    },

    #[error("Runner {runner_id} is not online (status: {status})")]
    RunnerNotOnline { runner_id: RunnerId, status: String },

    #[error(
        "Cannot promote solo project to team while {active_runs} active or queued run(s) exist"
    )]
    SoloToTeamBlockedByActiveRuns { active_runs: usize },

    #[error("Invalid invitation state: {0}")]
    InvalidInvitationState(String),
}

/// Generic domain-level error.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("Transition error: {0}")]
    Transition(#[from] TransitionError),

    #[error("Invalid entity state: {0}")]
    InvalidState(String),

    #[error("Invalid SHA-256 artifact digest: {0}")]
    InvalidDigest(String),
}
