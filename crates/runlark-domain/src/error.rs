use crate::id::{Revision, RunId, SessionId};
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
}

/// Generic domain-level error.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("Transition error: {0}")]
    Transition(#[from] TransitionError),

    #[error("Invalid entity state: {0}")]
    InvalidState(String),
}
