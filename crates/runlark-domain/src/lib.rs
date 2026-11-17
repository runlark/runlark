pub mod board_command;
pub mod budget;
pub mod chat;
pub mod command;
pub mod decision;
pub mod error;
pub mod event;
pub mod id;
pub mod project;
pub mod state;
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
pub use command::{
    AnswerQuestionCommand, ArchiveSessionCommand, ChangeCwdCommand, ResumeSessionCommand,
    SendPromptCommand, SessionCommand, StopRunCommand,
};
pub use decision::{Decision, DecisionBinding, DecisionKind, verify_decision_binding};
pub use error::{DomainError, TransitionError};
pub use event::DomainEvent;
pub use id::{
    ArtifactDigest, AttemptId, CommandId, DecisionId, EventSeq, PluginId, PolicyDigest, ProjectId,
    QuestionId, Revision, RunId, SessionId, StepId, TaskId, Timestamp,
};
pub use project::{ArtifactKind, ArtifactRef, DEFAULT_MAX_CONCURRENT_RUNS, Project};
pub use state::{Question, QuestionStatus, Run, RunStatus, Session, SessionStatus};
pub use task::{ActorRole, Task, TaskKind, TaskStage, WorkflowStage, validate_task_stage_move};
pub use transition::{
    DecisionOutcome, RepairScheduleOutcome, RuntimeEvent, StageTransitionOutcome,
    TaskTransitionOutcome, TransitionOutcome, TurnEndStatus, advance_run_stage, apply_decision,
    apply_runtime_event, move_task_stage, record_attempt_and_budget, schedule_repair,
    start_task_run, transition, update_task_plan,
};
