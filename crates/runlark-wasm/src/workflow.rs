use crate::error::WasmError;
use crate::wit_contract::WitContract;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Single allowed transition between declarative workflow stages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageTransition {
    pub from_stage: String,
    pub to_stage: String,
    pub max_attempts: u32,
}

/// Execution and spend budgets enforced for a workflow definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowBudget {
    pub max_repair_attempts: u32,
    pub max_wall_seconds: u64,
    pub max_spend_cents: u64,
    pub human_acceptance_required: bool,
}

/// Declarative workflow definition (`workflow.json`) shipped in a workflow package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub schema_version: u32,
    pub workflow_id: String,
    pub stages: Vec<String>,
    pub transitions: Vec<StageTransition>,
    pub validator_stage: String,
    pub budget: WorkflowBudget,
}

impl WorkflowDefinition {
    /// Validates workflow stages, transitions, budgets, and human-acceptance boundary
    /// against the WIT contract's `workflow-stage` enum.
    ///
    /// # Errors
    /// Returns [`WasmError::InvalidWorkflow`] if any stage, transition, or budget invariant fails.
    pub fn validate(&self, contract: &WitContract) -> Result<(), WasmError> {
        if self.schema_version != 1 {
            return Err(WasmError::InvalidWorkflow(format!(
                "unsupported workflow schema_version {}, expected 1",
                self.schema_version
            )));
        }
        if self.workflow_id.trim().is_empty() {
            return Err(WasmError::InvalidWorkflow(
                "workflow_id must not be empty".to_string(),
            ));
        }
        if self.stages.is_empty() {
            return Err(WasmError::InvalidWorkflow(
                "workflow must declare at least one stage".to_string(),
            ));
        }

        let allowed_stages = contract.allowed_stages();
        let mut declared = BTreeSet::new();
        for stage in &self.stages {
            if !allowed_stages.contains(stage) {
                return Err(WasmError::InvalidWorkflow(format!(
                    "stage `{stage}` is not defined in WIT `workflow-stage`"
                )));
            }
            if !declared.insert(stage.as_str()) {
                return Err(WasmError::InvalidWorkflow(format!(
                    "duplicate stage `{stage}` in workflow"
                )));
            }
        }

        if !declared.contains(self.validator_stage.as_str()) {
            return Err(WasmError::InvalidWorkflow(format!(
                "validator_stage `{}` is not listed in workflow stages",
                self.validator_stage
            )));
        }
        if !declared.contains("human-accept") {
            return Err(WasmError::InvalidWorkflow(
                "workflow must end in explicit `human-accept` stage".to_string(),
            ));
        }
        if self.stages.last().map(String::as_str) != Some("human-accept") {
            return Err(WasmError::InvalidWorkflow(
                "`human-accept` must be the terminal workflow stage".to_string(),
            ));
        }
        if !self.budget.human_acceptance_required {
            return Err(WasmError::InvalidWorkflow(
                "human_acceptance_required must be true (WASM/agent cannot auto-accept)"
                    .to_string(),
            ));
        }
        if self.budget.max_repair_attempts == 0 || self.budget.max_wall_seconds == 0 {
            return Err(WasmError::InvalidWorkflow(
                "repair attempts and wall time budgets must be positive".to_string(),
            ));
        }

        for tr in &self.transitions {
            if !declared.contains(tr.from_stage.as_str())
                || !declared.contains(tr.to_stage.as_str())
            {
                return Err(WasmError::InvalidWorkflow(format!(
                    "transition `{} -> {}` references undeclared stage",
                    tr.from_stage, tr.to_stage
                )));
            }
            if tr.from_stage == "human-accept" {
                return Err(WasmError::InvalidWorkflow(
                    "terminal `human-accept` stage cannot have outgoing transitions".to_string(),
                ));
            }
            if tr.max_attempts == 0 {
                return Err(WasmError::InvalidWorkflow(format!(
                    "transition `{} -> {}` must allow at least 1 attempt",
                    tr.from_stage, tr.to_stage
                )));
            }
        }

        Ok(())
    }
}
