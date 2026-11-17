use crate::id::{CommandId, EventSeq, RunId, SessionId, Timestamp};
use serde::{Deserialize, Serialize};

/// Immutable recorded facts within a workflow session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DomainEvent {
    SessionCreated {
        session_id: SessionId,
        cwd: String,
        timestamp: Timestamp,
    },
    RunCreated {
        run_id: RunId,
        session_id: SessionId,
        worktree_path: String,
        timestamp: Timestamp,
    },
    PromptEnqueued {
        run_id: RunId,
        command_id: CommandId,
        prompt: String,
        timestamp: Timestamp,
    },
    RunCancelRequested {
        run_id: RunId,
        command_id: CommandId,
        timestamp: Timestamp,
    },
    RunStopped {
        run_id: RunId,
        timestamp: Timestamp,
    },
    RunCompleted {
        run_id: RunId,
        timestamp: Timestamp,
    },
    RunFailed {
        run_id: RunId,
        error: String,
        timestamp: Timestamp,
    },
    RunInterrupted {
        run_id: RunId,
        reason: String,
        timestamp: Timestamp,
    },
    CwdChanged {
        session_id: SessionId,
        new_cwd: String,
        timestamp: Timestamp,
    },
    SessionArchived {
        session_id: SessionId,
        timestamp: Timestamp,
    },
    SessionResumed {
        session_id: SessionId,
        run_id: RunId,
        command_id: CommandId,
        after_seq: Option<EventSeq>,
        timestamp: Timestamp,
    },
}
