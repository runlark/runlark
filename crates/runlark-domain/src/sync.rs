use crate::board_command::{
    AttachArtifactCommand, CreateTaskCommand, MoveTaskStageCommand, RecordAttemptCommand,
    RecordDecisionCommand, UpdateTaskPlanCommand,
};
use crate::budget::Attempt;
use crate::collaboration::{
    RunOwnership, RunnerRegistration, TeamProjectSpec, Workspace, WorkspaceMember,
};
use crate::decision::Decision;
use crate::id::{
    CommandId, CommentId, CursorSeq, DraftId, EventSeq, ProjectId, Revision, RunId, RunnerId,
    TaskId, Timestamp, UserId, WorkspaceId,
};
use crate::project::ArtifactRef;
use crate::state::{Run, RunStatus};
use crate::task::{Task, WorkflowStage};
use serde::{Deserialize, Serialize};

/// Immutable append-only task discussion comment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskComment {
    pub id: CommentId,
    pub task_id: TaskId,
    pub run_id: Option<RunId>,
    pub author_user_id: UserId,
    pub author_display_name: String,
    pub body: String,
    pub created_at: Timestamp,
}

/// Command to append an immutable comment onto a task timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddCommentCommand {
    pub command_id: CommandId,
    pub comment_id: CommentId,
    pub task_id: TaskId,
    pub run_id: Option<RunId>,
    pub body: String,
    pub created_at: Timestamp,
}

/// Command to assign a single online runner to a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignRunnerCommand {
    pub command_id: CommandId,
    pub run_id: RunId,
    pub runner_id: RunnerId,
    pub expected_revision: Revision,
    pub created_at: Timestamp,
}

/// Command to explicitly cancel an unreachable runner's active run without auto-takeover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForceCancelUnreachableRunCommand {
    pub command_id: CommandId,
    pub run_id: RunId,
    pub reason: String,
    pub expected_revision: Revision,
    pub created_at: Timestamp,
}

/// Command to request cancellation of an active team run (`CancelRequested`, not `Stopped`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelTeamRunCommand {
    pub command_id: CommandId,
    pub run_id: RunId,
    pub reason: String,
    pub expected_revision: Revision,
    pub created_at: Timestamp,
}

/// Command to create a server-coordinated run entry for a team task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateTeamRunCommand {
    pub command_id: CommandId,
    pub run_id: RunId,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub runner_id: Option<RunnerId>,
    pub expected_revision: Revision,
    pub created_at: Timestamp,
}

/// Ordered team protocol commands processed by `runlark-server`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum TeamCommand {
    CreateTask(CreateTaskCommand),
    MoveTaskStage(MoveTaskStageCommand),
    UpdateTaskPlan(UpdateTaskPlanCommand),
    CreateTeamRun(CreateTeamRunCommand),
    AssignRunner(AssignRunnerCommand),
    CancelRun(CancelTeamRunCommand),
    ForceCancelUnreachableRun(ForceCancelUnreachableRunCommand),
    AddComment(AddCommentCommand),
    RecordAttempt(RecordAttemptCommand),
    RecordDecision(RecordDecisionCommand),
    AttachArtifact(AttachArtifactCommand),
}

impl TeamCommand {
    /// Returns the stable `CommandId` used for server-side idempotency deduplication.
    #[must_use]
    pub fn command_id(&self) -> &CommandId {
        match self {
            Self::CreateTask(c) => &c.command_id,
            Self::MoveTaskStage(c) => &c.command_id,
            Self::UpdateTaskPlan(c) => &c.command_id,
            Self::CreateTeamRun(c) => &c.command_id,
            Self::AssignRunner(c) => &c.command_id,
            Self::CancelRun(c) => &c.command_id,
            Self::ForceCancelUnreachableRun(c) => &c.command_id,
            Self::AddComment(c) => &c.command_id,
            Self::RecordAttempt(c) => &c.command_id,
            Self::RecordDecision(c) => &c.command_id,
            Self::AttachArtifact(c) => &c.command_id,
        }
    }

    /// Returns a canonical copy with `created_at` zeroed for retry-safe SHA-256 payload hashing.
    #[must_use]
    pub fn canonical_for_hash(&self) -> Self {
        let mut cloned = self.clone();
        match &mut cloned {
            Self::CreateTask(c) => c.created_at = Timestamp(0),
            Self::MoveTaskStage(c) => c.created_at = Timestamp(0),
            Self::UpdateTaskPlan(c) => c.created_at = Timestamp(0),
            Self::CreateTeamRun(c) => c.created_at = Timestamp(0),
            Self::AssignRunner(c) => c.created_at = Timestamp(0),
            Self::CancelRun(c) => c.created_at = Timestamp(0),
            Self::ForceCancelUnreachableRun(c) => c.created_at = Timestamp(0),
            Self::AddComment(c) => c.created_at = Timestamp(0),
            Self::RecordAttempt(c) => c.created_at = Timestamp(0),
            Self::RecordDecision(c) => c.created_at = Timestamp(0),
            Self::AttachArtifact(c) => c.created_at = Timestamp(0),
        }
        cloned
    }
}

/// Ordered workspace sync event payload emitted into `workspace_events`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum WorkspaceSyncEvent {
    ProjectPromoted {
        project: TeamProjectSpec,
    },
    TaskUpserted {
        task: Task,
    },
    RunUpserted {
        run: Run,
        ownership: RunOwnership,
    },
    CommentAppended {
        comment: TaskComment,
    },
    AttemptRecorded {
        attempt: Attempt,
    },
    DecisionRecorded {
        decision: Decision,
    },
    ArtifactLinked {
        run_id: RunId,
        artifact: ArtifactRef,
        label: String,
    },
    RunnerUpdated {
        runner: RunnerRegistration,
    },
    MemberUpdated {
        member: WorkspaceMember,
    },
}

/// Persisted workspace event envelope bound to a monotonic `CursorSeq`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEventEnvelope {
    pub cursor_seq: CursorSeq,
    pub workspace_id: WorkspaceId,
    pub project_id: Option<ProjectId>,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub event: WorkspaceSyncEvent,
    pub actor_user_id: UserId,
    pub created_at: Timestamp,
}

/// Server run state paired with its exclusive runner ownership lease.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamRunEntry {
    pub run: Run,
    pub ownership: RunOwnership,
}

/// Server artifact link entry included in workspace snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamRunArtifactEntry {
    pub run_id: RunId,
    pub artifact: ArtifactRef,
    pub label: String,
}

/// Consistent point-in-time snapshot whose `cursor_seq` matches all returned state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    pub cursor_seq: CursorSeq,
    pub workspace: Workspace,
    pub members: Vec<WorkspaceMember>,
    pub projects: Vec<TeamProjectSpec>,
    pub tasks: Vec<Task>,
    pub runs: Vec<TeamRunEntry>,
    pub attempts: Vec<Attempt>,
    pub decisions: Vec<Decision>,
    pub comments: Vec<TaskComment>,
    pub artifacts: Vec<TeamRunArtifactEntry>,
    pub runners: Vec<RunnerRegistration>,
}

/// Per-run ordered wire/domain event delivered by the owning runner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamRunEvent {
    pub run_id: RunId,
    pub seq: EventSeq,
    pub runner_id: RunnerId,
    pub lease_epoch: u64,
    pub event_type: String,
    pub payload_json: String,
    pub created_at: Timestamp,
}

/// Fenced status update submitted by the owning runner for its assigned run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerRunReport {
    pub runner_id: RunnerId,
    pub lease_epoch: u64,
    pub status: Option<RunStatus>,
    #[serde(default)]
    pub workflow_stage: Option<WorkflowStage>,
    pub head_sha: Option<String>,
    pub base_sha: Option<String>,
    pub events: Vec<RunnerEventInput>,
}

/// Event item inside [`RunnerRunReport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerEventInput {
    pub seq: EventSeq,
    pub event_type: String,
    pub payload_json: String,
    pub created_at: Timestamp,
}

/// Kind of client-side draft queued while offline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftKind {
    TaskCreate,
    TaskComment,
    PlanEdit,
    DecisionIntent,
}

/// Synchronization status of a local offline draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DraftSyncState {
    LocalDraft,
    PendingSync,
    Conflict {
        server_revision: Option<Revision>,
        reason: String,
    },
    Synced,
}

/// Local draft persisted in SQLite/browser storage until explicitly accepted or resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfflineDraft {
    pub id: DraftId,
    pub project_id: ProjectId,
    pub task_id: Option<TaskId>,
    pub kind: DraftKind,
    pub payload_json: String,
    pub base_revision: Option<Revision>,
    pub sync_state: DraftSyncState,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
