use crate::budget::{AttemptOutcome, BudgetPolicy};
use crate::decision::DecisionKind;
use crate::id::{
    ArtifactDigest, AttemptId, CommandId, DecisionId, PolicyDigest, ProjectId, Revision, RunId,
    TaskId, Timestamp,
};
use crate::project::ArtifactKind;
use crate::task::{ActorRole, TaskKind, TaskStage, WorkflowStage};
use serde::{Deserialize, Serialize};

/// Command to register a project workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateProjectCommand {
    pub command_id: CommandId,
    pub project_id: ProjectId,
    pub name: String,
    pub repo_root: String,
    pub default_branch: String,
    pub budget_policy: BudgetPolicy,
    pub policy_digest: PolicyDigest,
    pub created_at: Timestamp,
}

/// Command to create a Kanban task in `Inbox`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateTaskCommand {
    pub command_id: CommandId,
    pub task_id: TaskId,
    pub project_id: ProjectId,
    pub title: String,
    pub description: String,
    pub kind: TaskKind,
    pub base_ref: Option<String>,
    pub budget_override: Option<BudgetPolicy>,
    pub created_at: Timestamp,
}

/// Command to move a task across Kanban board columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveTaskStageCommand {
    pub command_id: CommandId,
    pub task_id: TaskId,
    pub target_stage: TaskStage,
    pub actor: ActorRole,
    pub expected_revision: Revision,
    pub created_at: Timestamp,
}

/// Command to update a task's plan text and bump `plan_version`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateTaskPlanCommand {
    pub command_id: CommandId,
    pub task_id: TaskId,
    pub plan_text: String,
    pub expected_revision: Revision,
    pub created_at: Timestamp,
}

/// Command to record an attempt outcome and cost ledger delta on a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordAttemptCommand {
    pub command_id: CommandId,
    pub attempt_id: AttemptId,
    pub run_id: RunId,
    pub workflow_stage: WorkflowStage,
    pub outcome: AttemptOutcome,
    pub cost_micro_usd: Option<u64>,
    pub unknown_cost_reason: Option<String>,
    pub repair_reason: Option<String>,
    pub expected_revision: Revision,
    pub created_at: Timestamp,
}

/// Command to record a bound human decision (`Accept`, `RequestChanges`, `ApprovePlan`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordDecisionCommand {
    pub command_id: CommandId,
    pub decision_id: DecisionId,
    pub task_id: TaskId,
    pub run_id: RunId,
    pub kind: DecisionKind,
    pub actor: String,
    pub plan_version: u32,
    pub head_sha: String,
    pub base_sha: String,
    pub policy_digest: PolicyDigest,
    pub evidence_digest: ArtifactDigest,
    pub comment: Option<String>,
    pub expected_revision: Revision,
    pub created_at: Timestamp,
}

/// Command to attach an immutable CAS artifact reference to a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachArtifactCommand {
    pub command_id: CommandId,
    pub run_id: RunId,
    pub digest_sha256: ArtifactDigest,
    pub kind: ArtifactKind,
    pub media_type: String,
    pub size_bytes: u64,
    pub label: String,
    pub created_at: Timestamp,
}

/// Enum of all A1 board, project, budget, decision, and artifact commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum BoardCommand {
    CreateProject(CreateProjectCommand),
    CreateTask(CreateTaskCommand),
    MoveTaskStage(MoveTaskStageCommand),
    UpdateTaskPlan(UpdateTaskPlanCommand),
    RecordAttempt(RecordAttemptCommand),
    RecordDecision(RecordDecisionCommand),
    AttachArtifact(AttachArtifactCommand),
}

impl BoardCommand {
    /// Returns the stable idempotency `CommandId` for this board command.
    #[must_use]
    pub fn command_id(&self) -> &CommandId {
        match self {
            Self::CreateProject(c) => &c.command_id,
            Self::CreateTask(c) => &c.command_id,
            Self::MoveTaskStage(c) => &c.command_id,
            Self::UpdateTaskPlan(c) => &c.command_id,
            Self::RecordAttempt(c) => &c.command_id,
            Self::RecordDecision(c) => &c.command_id,
            Self::AttachArtifact(c) => &c.command_id,
        }
    }

    /// Returns a canonical copy of the command with `created_at` normalized to `Timestamp(0)`
    /// for idempotent payload hashing across retries.
    #[must_use]
    pub fn canonical_for_hash(&self) -> Self {
        let mut cloned = self.clone();
        match &mut cloned {
            Self::CreateProject(c) => c.created_at = Timestamp(0),
            Self::CreateTask(c) => c.created_at = Timestamp(0),
            Self::MoveTaskStage(c) => c.created_at = Timestamp(0),
            Self::UpdateTaskPlan(c) => c.created_at = Timestamp(0),
            Self::RecordAttempt(c) => c.created_at = Timestamp(0),
            Self::RecordDecision(c) => c.created_at = Timestamp(0),
            Self::AttachArtifact(c) => c.created_at = Timestamp(0),
        }
        cloned
    }
}
