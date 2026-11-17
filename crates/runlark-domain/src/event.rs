use crate::decision::DecisionKind;
use crate::id::{
    ArtifactDigest, AttemptId, CommandId, DecisionId, EventSeq, PolicyDigest, ProjectId,
    QuestionId, RunId, SessionId, TaskId, Timestamp,
};
use crate::project::ArtifactKind;
use crate::task::{TaskKind, TaskStage, WorkflowStage};
use serde::{Deserialize, Serialize};

/// Immutable recorded facts within a workflow session or board task.
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
    QuestionAsked {
        question_id: QuestionId,
        session_id: SessionId,
        run_id: RunId,
        prompt: String,
        options: Vec<String>,
        timestamp: Timestamp,
    },
    QuestionAnswered {
        question_id: QuestionId,
        session_id: SessionId,
        run_id: RunId,
        command_id: CommandId,
        answer: String,
        timestamp: Timestamp,
    },
    RunCancelRequested {
        run_id: RunId,
        command_id: CommandId,
        reason: Option<String>,
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
    ProjectCreated {
        project_id: ProjectId,
        name: String,
        policy_digest: PolicyDigest,
        timestamp: Timestamp,
    },
    TaskCreated {
        task_id: TaskId,
        project_id: ProjectId,
        title: String,
        kind: TaskKind,
        timestamp: Timestamp,
    },
    TaskStageChanged {
        task_id: TaskId,
        from: TaskStage,
        to: TaskStage,
        timestamp: Timestamp,
    },
    TaskPlanUpdated {
        task_id: TaskId,
        plan_version: u32,
        timestamp: Timestamp,
    },
    RunStageAdvanced {
        run_id: RunId,
        from: WorkflowStage,
        to: WorkflowStage,
        timestamp: Timestamp,
    },
    AttemptRecorded {
        attempt_id: AttemptId,
        run_id: RunId,
        attempt_no: u32,
        workflow_stage: WorkflowStage,
        cost_micro_usd: Option<u64>,
        timestamp: Timestamp,
    },
    RepairScheduled {
        run_id: RunId,
        repair_attempt: u32,
        reason: String,
        timestamp: Timestamp,
    },
    BudgetExceeded {
        run_id: RunId,
        reason: String,
        timestamp: Timestamp,
    },
    UnknownCostRecorded {
        run_id: RunId,
        attempt_id: AttemptId,
        reason: String,
        timestamp: Timestamp,
    },
    DecisionRecorded {
        decision_id: DecisionId,
        task_id: TaskId,
        run_id: RunId,
        kind: DecisionKind,
        actor: String,
        plan_version: u32,
        head_sha: String,
        evidence_digest: ArtifactDigest,
        timestamp: Timestamp,
    },
    ArtifactAttached {
        run_id: RunId,
        digest_sha256: ArtifactDigest,
        kind: ArtifactKind,
        label: String,
        timestamp: Timestamp,
    },
}
