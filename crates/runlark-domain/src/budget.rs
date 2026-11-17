use crate::id::{AttemptId, RunId, Timestamp};
use crate::state::Run;
use crate::task::WorkflowStage;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Execution budget limits for a project, task, or run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetPolicy {
    pub max_attempts: u32,
    pub max_duration_ms: u64,
    pub max_spend_micro_usd: u64,
    pub allow_unknown_cost_repair: bool,
}

impl Default for BudgetPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            max_duration_ms: 1_800_000,
            max_spend_micro_usd: 5_000_000,
            allow_unknown_cost_repair: false,
        }
    }
}

/// Outcome of a single attempt within a run stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AttemptOutcome {
    InProgress,
    Succeeded,
    Failed { error: String },
    Stopped,
    Unknown { reason: String },
}

/// Durable record of a single execution or repair attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub id: AttemptId,
    pub run_id: RunId,
    pub attempt_no: u32,
    pub workflow_stage: WorkflowStage,
    pub started_at: Timestamp,
    pub ended_at: Option<Timestamp>,
    pub outcome: AttemptOutcome,
    pub cost_micro_usd: Option<u64>,
    pub unknown_cost_reason: Option<String>,
    pub repair_reason: Option<String>,
}

/// Explicit cost ledger keeping known, held, and unpriced unknown attempts separate.
///
/// INVARIANT: An attempt with `cost_micro_usd == None` increments `unknown_attempts` and is
/// NEVER folded into `0`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CostSummary {
    pub known_micro_usd: u64,
    pub held_micro_usd: u64,
    pub unknown_attempts: u32,
    #[serde(default)]
    pub unknown_reasons: Vec<String>,
}

impl CostSummary {
    /// Folds a slice of attempts and any held spend reservation into an explicit [`CostSummary`].
    #[must_use]
    pub fn from_attempts(attempts: &[Attempt], held_micro_usd: u64) -> Self {
        let mut summary = Self {
            known_micro_usd: 0,
            held_micro_usd,
            unknown_attempts: 0,
            unknown_reasons: Vec::new(),
        };
        for attempt in attempts {
            summary.record_attempt(
                attempt.cost_micro_usd,
                attempt.unknown_cost_reason.as_deref(),
            );
        }
        summary
    }

    /// Records a single attempt's cost (`Some` adds to `known_micro_usd`; `None` increments
    /// `unknown_attempts` rather than treating unknown cost as zero).
    pub fn record_attempt(&mut self, cost_micro_usd: Option<u64>, reason: Option<&str>) {
        if let Some(micro_usd) = cost_micro_usd {
            self.known_micro_usd = self.known_micro_usd.saturating_add(micro_usd);
        } else {
            self.unknown_attempts = self.unknown_attempts.saturating_add(1);
            if let Some(msg) = reason.filter(|s| !s.trim().is_empty()) {
                self.unknown_reasons.push(msg.to_string());
            }
        }
    }

    /// Returns `true` only when every recorded attempt has a known cost.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.unknown_attempts == 0
    }

    /// Returns total committed plus held micro-USD (excluding unknown attempts).
    #[must_use]
    pub const fn total_committed_and_held(&self) -> u64 {
        self.known_micro_usd.saturating_add(self.held_micro_usd)
    }
}

/// Reason a bounded repair retry is denied by budget policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
pub enum BudgetBlockReason {
    #[error("maximum attempts reached ({used}/{max})")]
    MaxAttemptsReached { used: u32, max: u32 },

    #[error("maximum duration exceeded ({elapsed_ms}ms >= {max_ms}ms)")]
    MaxDurationExceeded { elapsed_ms: u64, max_ms: u64 },

    #[error("maximum spend exceeded ({total_micro_usd} >= {max_micro_usd} micro-USD)")]
    MaxSpendExceeded {
        total_micro_usd: u64,
        max_micro_usd: u64,
    },

    #[error("repair blocked by {unknown_attempts} attempt(s) with unknown cost")]
    UnknownCostBlocked { unknown_attempts: u32 },
}

/// Evaluates whether a run is eligible for another bounded repair attempt.
///
/// # Errors
/// Returns [`BudgetBlockReason`] when attempts, elapsed duration, known+held spend, or
/// unpriced unknown-cost limits are exceeded.
pub fn can_retry_repair(
    run: &Run,
    attempts: &[Attempt],
    policy: &BudgetPolicy,
    elapsed_ms: u64,
) -> Result<(), BudgetBlockReason> {
    let attempts_used = u32::try_from(attempts.len())
        .unwrap_or(u32::MAX)
        .max(run.repair_attempts);
    if attempts_used >= policy.max_attempts {
        return Err(BudgetBlockReason::MaxAttemptsReached {
            used: attempts_used,
            max: policy.max_attempts,
        });
    }

    if elapsed_ms >= policy.max_duration_ms {
        return Err(BudgetBlockReason::MaxDurationExceeded {
            elapsed_ms,
            max_ms: policy.max_duration_ms,
        });
    }

    let summary = if attempts.is_empty() {
        run.cost_summary.clone()
    } else {
        CostSummary::from_attempts(attempts, run.cost_summary.held_micro_usd)
    };

    if !summary.is_complete() && !policy.allow_unknown_cost_repair {
        return Err(BudgetBlockReason::UnknownCostBlocked {
            unknown_attempts: summary.unknown_attempts,
        });
    }

    let total_spend = summary.total_committed_and_held();
    if total_spend >= policy.max_spend_micro_usd {
        return Err(BudgetBlockReason::MaxSpendExceeded {
            total_micro_usd: total_spend,
            max_micro_usd: policy.max_spend_micro_usd,
        });
    }

    Ok(())
}
