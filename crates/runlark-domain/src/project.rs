use crate::budget::BudgetPolicy;
use crate::id::{ArtifactDigest, PolicyDigest, ProjectId, Revision, Timestamp};
use serde::{Deserialize, Serialize};

/// Default solo-alpha maximum concurrent executing runs per project.
pub const DEFAULT_MAX_CONCURRENT_RUNS: u32 = 2;

/// Project workspace configuration and default execution budgets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub repo_root: String,
    pub default_branch: String,
    pub max_concurrent_runs: u32,
    pub budget_policy: BudgetPolicy,
    pub policy_digest: PolicyDigest,
    pub revision: Revision,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Project {
    /// Constructs a new project with the solo-alpha 2-slot concurrency limit.
    #[must_use]
    pub fn new(
        id: ProjectId,
        name: impl Into<String>,
        repo_root: impl Into<String>,
        default_branch: impl Into<String>,
        budget_policy: BudgetPolicy,
        policy_digest: PolicyDigest,
        now: Timestamp,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            repo_root: repo_root.into(),
            default_branch: default_branch.into(),
            max_concurrent_runs: DEFAULT_MAX_CONCURRENT_RUNS,
            budget_policy,
            policy_digest,
            revision: Revision(1),
            created_at: now,
            updated_at: now,
        }
    }
}

/// Classification of immutable CAS-backed run artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Diff,
    VerificationLog,
    WasmValidation,
    Plan,
    EvidenceBundle,
}

/// Content-addressed artifact metadata record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub digest_sha256: ArtifactDigest,
    pub kind: ArtifactKind,
    pub media_type: String,
    pub size_bytes: u64,
    pub created_at: Timestamp,
}
