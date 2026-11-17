use crate::error::TransitionError;
use crate::id::{ArtifactDigest, DecisionId, PolicyDigest, RunId, TaskId, Timestamp};
use serde::{Deserialize, Serialize};

/// Type of human review or planning decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    Accept,
    RequestChanges,
    ApprovePlan,
}

/// Immutable human decision bound to exact plan, Git SHAs, policy, and evidence digests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub id: DecisionId,
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
    pub created_at: Timestamp,
}

/// Expected binding snapshot checked before recording or consuming a [`Decision`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecisionBinding<'a> {
    pub plan_version: u32,
    pub head_sha: &'a str,
    pub base_sha: &'a str,
    pub policy_digest: &'a PolicyDigest,
    pub evidence_digest: &'a ArtifactDigest,
}

/// Verifies that a [`Decision`] matches the exact current `DecisionBinding` before downstream
/// effects (like PR creation) execute.
///
/// # Errors
/// Returns [`TransitionError::MissingDecisionBinding`] if any required SHA/digest is blank,
/// or [`TransitionError::DecisionBindingMismatch`] if any bound field drifted.
pub fn verify_decision_binding(
    decision: &Decision,
    expected: &DecisionBinding<'_>,
) -> Result<(), TransitionError> {
    if decision.actor.trim().is_empty() {
        return Err(TransitionError::AgentCannotSelfAccept);
    }
    if decision.head_sha.trim().is_empty() {
        return Err(TransitionError::MissingDecisionBinding {
            field: "head_sha".to_string(),
        });
    }
    if decision.base_sha.trim().is_empty() {
        return Err(TransitionError::MissingDecisionBinding {
            field: "base_sha".to_string(),
        });
    }
    if decision.policy_digest.as_ref().trim().is_empty() {
        return Err(TransitionError::MissingDecisionBinding {
            field: "policy_digest".to_string(),
        });
    }

    if decision.plan_version != expected.plan_version {
        return Err(TransitionError::DecisionBindingMismatch {
            field: "plan_version".to_string(),
            expected: expected.plan_version.to_string(),
            actual: decision.plan_version.to_string(),
        });
    }
    if decision.head_sha != expected.head_sha {
        return Err(TransitionError::DecisionBindingMismatch {
            field: "head_sha".to_string(),
            expected: expected.head_sha.to_string(),
            actual: decision.head_sha.clone(),
        });
    }
    if decision.base_sha != expected.base_sha {
        return Err(TransitionError::DecisionBindingMismatch {
            field: "base_sha".to_string(),
            expected: expected.base_sha.to_string(),
            actual: decision.base_sha.clone(),
        });
    }
    if &decision.policy_digest != expected.policy_digest {
        return Err(TransitionError::DecisionBindingMismatch {
            field: "policy_digest".to_string(),
            expected: expected.policy_digest.to_string(),
            actual: decision.policy_digest.to_string(),
        });
    }
    if &decision.evidence_digest != expected.evidence_digest {
        return Err(TransitionError::DecisionBindingMismatch {
            field: "evidence_digest".to_string(),
            expected: expected.evidence_digest.to_string(),
            actual: decision.evidence_digest.to_string(),
        });
    }

    Ok(())
}
