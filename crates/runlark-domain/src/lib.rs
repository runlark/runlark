pub mod board_command;
pub mod budget;
pub mod chat;
pub mod collaboration;
pub mod command;
pub mod decision;
pub mod error;
pub mod event;
pub mod id;
pub mod project;
pub mod state;
pub mod sync;
pub mod task;
pub mod transition;

pub use board_command::{
    AttachArtifactCommand, BoardCommand, CreateProjectCommand, CreateTaskCommand,
    MoveTaskStageCommand, RecordAttemptCommand, RecordDecisionCommand, UpdateTaskPlanCommand,
};
pub use budget::{
    Attempt, AttemptOutcome, BudgetBlockReason, BudgetPolicy, CostSummary, can_retry_repair,
};
pub use chat::{ChatParseError, ParsedChatCommand, parse_chat_command};
pub use collaboration::{
    InvitationStatus, ProjectOwnershipMode, RunOwnership, RunnerRegistration, RunnerStatus,
    ServerPromotionAck, TeamProjectSpec, Workspace, WorkspaceAction, WorkspaceInvitation,
    WorkspaceMember, WorkspaceRole, authorize_workspace_action, sanitize_project_for_team_snapshot,
    validate_runner_assignment, validate_runner_fencing, validate_solo_to_team_promotion,
};
pub use command::{
    AnswerQuestionCommand, ArchiveSessionCommand, ChangeCwdCommand, ResumeSessionCommand,
    SendPromptCommand, SessionCommand, StopRunCommand,
};
pub use decision::{Decision, DecisionBinding, DecisionKind, verify_decision_binding};
pub use error::{DomainError, TransitionError};
pub use event::DomainEvent;
pub use id::{
    ArtifactDigest, AttemptId, CommandId, CommentId, CursorSeq, DecisionId, DraftId, EventSeq,
    InviteId, LeaseEpoch, PluginId, PolicyDigest, ProjectId, QuestionId, Revision, RunId, RunnerId,
    SessionId, StepId, TaskId, Timestamp, UserId, WorkspaceId,
};
pub use project::{ArtifactKind, ArtifactRef, DEFAULT_MAX_CONCURRENT_RUNS, Project};
pub use state::{Question, QuestionStatus, Run, RunStatus, Session, SessionStatus};
pub use sync::{
    AddCommentCommand, AssignRunnerCommand, CancelTeamRunCommand, CreateTeamRunCommand, DraftKind,
    DraftSyncState, ForceCancelUnreachableRunCommand, OfflineDraft, RunnerEventInput,
    RunnerRunReport, TaskComment, TeamCommand, TeamRunArtifactEntry, TeamRunEntry, TeamRunEvent,
    WorkspaceEventEnvelope, WorkspaceSnapshot, WorkspaceSyncEvent,
};
pub use task::{ActorRole, Task, TaskKind, TaskStage, WorkflowStage, validate_task_stage_move};
pub use transition::{
    DecisionOutcome, RepairScheduleOutcome, RuntimeEvent, StageTransitionOutcome,
    TaskTransitionOutcome, TransitionOutcome, TurnEndStatus, advance_run_stage, apply_decision,
    apply_runtime_event, move_task_stage, record_attempt_and_budget, schedule_repair,
    start_task_run, transition, update_task_plan,
};
