use crate::id::{QuestionId, Revision, RunId, SessionId, TaskId, Timestamp};
use serde::{Deserialize, Serialize};

/// Lifecycle status of an agent session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SessionStatus {
    Created,
    Active,
    Archived,
}

/// Lifecycle status of a workflow run execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RunStatus {
    Queued,
    Running,
    WaitingForApproval,
    CancelRequested,
    Stopped,
    Completed,
    Failed,
    Interrupted,
}

/// Lifecycle status of a human question or approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum QuestionStatus {
    Pending,
    Answered,
    Cancelled,
}

/// Managed session durable state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub task_id: TaskId,
    pub cwd: String,
    pub status: SessionStatus,
    pub current_revision: Revision,
    pub active_run_id: Option<RunId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Run execution durable state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub id: RunId,
    pub session_id: SessionId,
    pub status: RunStatus,
    pub worktree_path: String,
    pub revision: Revision,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Human question or approval request durable state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub id: QuestionId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub prompt: String,
    pub options: Vec<String>,
    pub status: QuestionStatus,
    pub answer: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
