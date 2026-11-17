use crate::error::StorageError;
use runlark_domain::{ArtifactKind, DecisionKind, TaskKind, TaskStage, WorkflowStage};
use sha2::{Digest, Sha256};

#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[must_use]
pub(crate) const fn task_kind_to_str(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::QuickFix => "quick_fix",
        TaskKind::Standard => "standard",
    }
}

pub(crate) fn str_to_task_kind(s: &str) -> Result<TaskKind, StorageError> {
    match s {
        "quick_fix" => Ok(TaskKind::QuickFix),
        "standard" => Ok(TaskKind::Standard),
        other => Err(StorageError::InvalidState(format!(
            "unknown task_kind `{other}`"
        ))),
    }
}

#[must_use]
pub(crate) const fn task_stage_to_str(stage: TaskStage) -> &'static str {
    match stage {
        TaskStage::Inbox => "inbox",
        TaskStage::Ready => "ready",
        TaskStage::InProgress => "in_progress",
        TaskStage::Acceptance => "acceptance",
        TaskStage::Done => "done",
    }
}

pub(crate) fn str_to_task_stage(s: &str) -> Result<TaskStage, StorageError> {
    match s {
        "inbox" => Ok(TaskStage::Inbox),
        "ready" => Ok(TaskStage::Ready),
        "in_progress" => Ok(TaskStage::InProgress),
        "acceptance" => Ok(TaskStage::Acceptance),
        "done" => Ok(TaskStage::Done),
        other => Err(StorageError::InvalidState(format!(
            "unknown task stage `{other}`"
        ))),
    }
}

#[must_use]
pub(crate) const fn workflow_stage_to_str(stage: WorkflowStage) -> &'static str {
    match stage {
        WorkflowStage::Clarification => "clarification",
        WorkflowStage::Plan => "plan",
        WorkflowStage::Implementation => "implementation",
        WorkflowStage::Verification => "verification",
        WorkflowStage::HumanAcceptance => "human_acceptance",
    }
}

pub(crate) fn str_to_workflow_stage(s: &str) -> Result<WorkflowStage, StorageError> {
    match s {
        "clarification" => Ok(WorkflowStage::Clarification),
        "plan" => Ok(WorkflowStage::Plan),
        "implementation" => Ok(WorkflowStage::Implementation),
        "verification" => Ok(WorkflowStage::Verification),
        "human_acceptance" => Ok(WorkflowStage::HumanAcceptance),
        other => Err(StorageError::InvalidState(format!(
            "unknown workflow_stage `{other}`"
        ))),
    }
}

#[must_use]
pub(crate) const fn decision_kind_to_str(kind: DecisionKind) -> &'static str {
    match kind {
        DecisionKind::Accept => "accept",
        DecisionKind::RequestChanges => "request_changes",
        DecisionKind::ApprovePlan => "approve_plan",
    }
}

pub(crate) fn str_to_decision_kind(s: &str) -> Result<DecisionKind, StorageError> {
    match s {
        "accept" => Ok(DecisionKind::Accept),
        "request_changes" => Ok(DecisionKind::RequestChanges),
        "approve_plan" => Ok(DecisionKind::ApprovePlan),
        other => Err(StorageError::InvalidState(format!(
            "unknown decision kind `{other}`"
        ))),
    }
}

#[must_use]
pub(crate) const fn artifact_kind_to_str(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Diff => "diff",
        ArtifactKind::VerificationLog => "verification_log",
        ArtifactKind::WasmValidation => "wasm_validation",
        ArtifactKind::Plan => "plan",
        ArtifactKind::EvidenceBundle => "evidence_bundle",
    }
}

pub(crate) fn str_to_artifact_kind(s: &str) -> Result<ArtifactKind, StorageError> {
    match s {
        "diff" => Ok(ArtifactKind::Diff),
        "verification_log" => Ok(ArtifactKind::VerificationLog),
        "wasm_validation" => Ok(ArtifactKind::WasmValidation),
        "plan" => Ok(ArtifactKind::Plan),
        "evidence_bundle" => Ok(ArtifactKind::EvidenceBundle),
        other => Err(StorageError::InvalidState(format!(
            "unknown artifact kind `{other}`"
        ))),
    }
}
