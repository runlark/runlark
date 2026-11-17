use crate::budget::{BudgetPolicy, CostSummary};
use crate::id::{
    ArtifactDigest, PolicyDigest, ProjectId, QuestionId, Revision, RunId, SessionId, TaskId,
    Timestamp,
};
use crate::task::{TaskKind, WorkflowStage};
use serde::{Deserialize, Serialize};

/// Lifecycle status of an agent session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SessionStatus {
    Created,
    Active,
    Archived,
}

/// Lifecycle status of a workflow run execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum RunStatus {
    #[default]
    Queued,
    Running,
    WaitingForApproval,
    CancelRequested,
    Stopped,
    Completed,
    Failed,
    Interrupted,
}

impl RunStatus {
    /// Returns `true` if a run in this status occupies one of the concurrent execution slots.
    ///
    /// `WaitingForApproval` intentionally frees its execution slot while paused for human input;
    /// `CancelRequested` retains its slot until the runtime confirms termination.
    #[must_use]
    pub const fn consumes_execution_slot(self) -> bool {
        matches!(self, Self::Running | Self::CancelRequested)
    }

    /// Returns `true` if the run has reached a terminal state.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Stopped | Self::Completed | Self::Failed)
    }
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
    #[serde(default)]
    pub task_id: Option<TaskId>,
    #[serde(default)]
    pub project_id: Option<ProjectId>,
    #[serde(default)]
    pub workflow_stage: WorkflowStage,
    #[serde(default)]
    pub task_kind: TaskKind,
    #[serde(default)]
    pub workflow_digest: Option<ArtifactDigest>,
    #[serde(default)]
    pub base_sha: Option<String>,
    #[serde(default)]
    pub head_sha: Option<String>,
    #[serde(default = "default_plan_version")]
    pub plan_version: u32,
    #[serde(default)]
    pub policy_digest: Option<PolicyDigest>,
    #[serde(default)]
    pub budget_policy: BudgetPolicy,
    #[serde(default)]
    pub cost_summary: CostSummary,
    #[serde(default)]
    pub repair_attempts: u32,
    #[serde(default)]
    pub pr_url: Option<String>,
}

const fn default_plan_version() -> u32 {
    1
}

impl Default for Run {
    fn default() -> Self {
        Self {
            id: RunId::from(""),
            session_id: SessionId::from(""),
            status: RunStatus::Queued,
            worktree_path: String::new(),
            revision: Revision(1),
            created_at: Timestamp(0),
            updated_at: Timestamp(0),
            task_id: None,
            project_id: None,
            workflow_stage: WorkflowStage::Implementation,
            task_kind: TaskKind::Standard,
            workflow_digest: None,
            base_sha: None,
            head_sha: None,
            plan_version: 1,
            policy_digest: None,
            budget_policy: BudgetPolicy::default(),
            cost_summary: CostSummary::default(),
            repair_attempts: 0,
            pr_url: None,
        }
    }
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
