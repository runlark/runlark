use crate::error::WasmError;
use crate::manifest::{PackageDigest, ResourceLimits, VerifiedPackage, sha256_hex};
use crate::wit_contract::{
    EXPECTED_TYPES_INTERFACE, EXPECTED_VALIDATOR_INTERFACE, EXPECTED_VALIDATOR_WORLD, WitContract,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder, Trap};

mod bindings {
    wasmtime::component::bindgen!({
        path: "../../wit/runlark-workflow.wit",
        world: "workflow-validator",
    });
}

use bindings::WorkflowValidator;
use bindings::runlark::workflow::types as wit_types;

/// Host-side representation of a workflow stage matching WIT `workflow-stage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowStage {
    Clarify,
    Plan,
    Implement,
    Validate,
    HumanAccept,
}

impl From<WorkflowStage> for wit_types::WorkflowStage {
    fn from(stage: WorkflowStage) -> Self {
        match stage {
            WorkflowStage::Clarify => Self::Clarify,
            WorkflowStage::Plan => Self::Plan,
            WorkflowStage::Implement => Self::Implement,
            WorkflowStage::Validate => Self::Validate,
            WorkflowStage::HumanAccept => Self::HumanAccept,
        }
    }
}

/// Host-side check evidence record passed into the WASM validator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckEvidence {
    pub name: String,
    pub passed: bool,
    pub exit_code: i32,
}

/// Input payload passed to `runlark:workflow/validator@0.1.0#validate-run`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationInput {
    pub task_id: String,
    pub run_id: String,
    pub stage: WorkflowStage,
    pub changed_files: Vec<String>,
    pub checks: Vec<CheckEvidence>,
    pub diff_bytes: u64,
    pub repair_attempt: u32,
}

/// Pass/fail verdict returned by the WASM validator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationVerdict {
    Pass,
    Fail,
}

/// Log entry emitted by the WASM component via the explicit `log` host import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostLogEntry {
    pub level: String,
    pub message: String,
}

/// Full execution report from running a digest-verified WASM validator component.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValidationReport {
    pub package_name: String,
    pub package_version: String,
    pub package_digest: PackageDigest,
    pub component_sha256: String,
    pub verdict: ValidationVerdict,
    pub summary: String,
    pub findings: Vec<String>,
    pub logs: Vec<HostLogEntry>,
    pub fuel_consumed: u64,
    pub fuel_limit: u64,
}

/// Component Model inspection metadata extracted before instantiation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComponentInspection {
    pub imports: Vec<String>,
    pub exports: Vec<String>,
    pub component_sha256: String,
}

struct ValidatorStoreState {
    limits: StoreLimits,
    logs: Vec<HostLogEntry>,
    allow_log: bool,
}

impl wit_types::Host for ValidatorStoreState {}

impl bindings::WorkflowValidatorImports for ValidatorStoreState {
    fn log(&mut self, level: String, message: String) {
        if self.allow_log && self.logs.len() < 64 {
            tracing::debug!(target: "runlark_wasm::guest", %level, %message, "validator log");
            self.logs.push(HostLogEntry { level, message });
        }
    }
}

/// Bounded Wasmtime Component Model host for executing verified Runlark validators.
pub struct WasmValidatorHost {
    engine: Engine,
}

impl WasmValidatorHost {
    /// Creates a new Wasmtime host configured with Component Model and fuel metering enabled.
    ///
    /// # Errors
    /// Returns [`WasmError::Execution`] if constructing the Wasmtime engine fails.
    pub fn new() -> Result<Self, WasmError> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.consume_fuel(true);
        let engine = Engine::new(&config)
            .map_err(|err| WasmError::Execution(format!("engine init failed: {err}")))?;
        Ok(Self { engine })
    }

    /// Compiles and validates a Component Model binary against `world workflow-validator`
    /// and a package's declared capability allowlist without instantiating or running guest code.
    ///
    /// # Errors
    /// Returns [`WasmError::IncompatibleComponent`] if compilation fails, undeclared imports
    /// (such as `wasi:*`) are present, or the required validator export is missing.
    pub fn inspect_component(
        &self,
        component_bytes: &[u8],
        declared_capabilities: &[String],
    ) -> Result<ComponentInspection, WasmError> {
        let component = Component::from_binary(&self.engine, component_bytes).map_err(|err| {
            WasmError::IncompatibleComponent(format!("invalid WASM component binary: {err}"))
        })?;
        let inspection =
            self.inspect_loaded_component(&component, component_bytes, declared_capabilities)?;
        Ok(inspection)
    }

    /// Re-verifies package digests, inspects component imports/exports, instantiates the
    /// component inside bounded memory/fuel limits, and executes `validate-run`.
    ///
    /// # Errors
    /// Returns [`WasmError`] if digest verification fails, imports/exports are incompatible,
    /// fuel/memory limits are exceeded, or the guest returns an explicit `Err(string)`.
    pub fn validate_run(
        &self,
        package: &VerifiedPackage,
        input: &ValidationInput,
    ) -> Result<ValidationReport, WasmError> {
        package.verify_digests()?;
        let component =
            Component::from_binary(&self.engine, &package.component_bytes).map_err(|err| {
                WasmError::IncompatibleComponent(format!("invalid WASM component binary: {err}"))
            })?;
        self.inspect_loaded_component(
            &component,
            &package.component_bytes,
            &package.manifest.capabilities,
        )?;

        let limits = package.manifest.limits.clamped();
        let store_limits = StoreLimitsBuilder::new()
            .memory_size(limits.max_memory_bytes)
            .tables(limits.max_tables)
            .instances(limits.max_instances)
            .trap_on_grow_failure(true)
            .build();

        let allow_log = package.manifest.capabilities.iter().any(|c| c == "log");
        let state = ValidatorStoreState {
            limits: store_limits,
            logs: Vec::new(),
            allow_log,
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        store
            .set_fuel(limits.max_fuel)
            .map_err(|err| WasmError::Execution(format!("failed to set fuel: {err}")))?;

        let mut linker: Linker<ValidatorStoreState> = Linker::new(&self.engine);
        WorkflowValidator::add_to_linker::<_, HasSelf<ValidatorStoreState>>(&mut linker, |s| s)
            .map_err(|err| WasmError::Execution(format!("linker binding failed: {err}")))?;

        let instance = WorkflowValidator::instantiate(&mut store, &component, &linker)
            .map_err(|err| map_wasmtime_error("instantiation failed", &err))?;

        let wit_input = wit_types::ValidationInput {
            task_id: input.task_id.clone(),
            run_id: input.run_id.clone(),
            stage: input.stage.into(),
            changed_files: input.changed_files.clone(),
            checks: input
                .checks
                .iter()
                .map(|c| wit_types::CheckEvidence {
                    name: c.name.clone(),
                    passed: c.passed,
                    exit_code: c.exit_code,
                })
                .collect(),
            diff_bytes: input.diff_bytes,
            repair_attempt: input.repair_attempt,
        };

        let call_result = instance
            .runlark_workflow_validator()
            .call_validate_run(&mut store, &wit_input)
            .map_err(|err| map_wasmtime_error("validate-run trap", &err))?;

        let remaining_fuel = store.get_fuel().unwrap_or(0);
        let fuel_consumed = limits.max_fuel.saturating_sub(remaining_fuel);
        let logs = std::mem::take(&mut store.data_mut().logs);

        match call_result {
            Ok(res) => {
                let verdict = match res.verdict {
                    wit_types::Verdict::Pass => ValidationVerdict::Pass,
                    wit_types::Verdict::Fail => ValidationVerdict::Fail,
                };
                Ok(ValidationReport {
                    package_name: package.manifest.name.clone(),
                    package_version: package.manifest.version.clone(),
                    package_digest: package.package_digest.clone(),
                    component_sha256: package.manifest.sha256.clone(),
                    verdict,
                    summary: res.summary,
                    findings: res.findings,
                    logs,
                    fuel_consumed,
                    fuel_limit: limits.max_fuel,
                })
            }
            Err(msg) => Err(WasmError::ValidatorRejected(msg)),
        }
    }

    fn inspect_loaded_component(
        &self,
        component: &Component,
        raw_bytes: &[u8],
        declared_capabilities: &[String],
    ) -> Result<ComponentInspection, WasmError> {
        let cap_set: BTreeSet<&str> = declared_capabilities.iter().map(String::as_str).collect();
        let comp_ty = component.component_type();

        let mut imports = Vec::new();
        for (name, _item) in comp_ty.imports(&self.engine) {
            if name == "log" {
                if !cap_set.contains("log") {
                    return Err(WasmError::IncompatibleComponent(
                        "component imports `log`, but manifest does not declare `log` capability"
                            .to_string(),
                    ));
                }
            } else if name != EXPECTED_TYPES_INTERFACE {
                return Err(WasmError::IncompatibleComponent(format!(
                    "disallowed component import `{name}` (no ambient WASI/FS/network permitted)"
                )));
            }
            imports.push(name.to_string());
        }

        let mut exports = Vec::new();
        let mut has_validator_export = false;
        for (name, _item) in comp_ty.exports(&self.engine) {
            if name == EXPECTED_VALIDATOR_INTERFACE {
                has_validator_export = true;
            }
            exports.push(name.to_string());
        }

        if !has_validator_export {
            return Err(WasmError::IncompatibleComponent(format!(
                "component missing required export `{EXPECTED_VALIDATOR_INTERFACE}`"
            )));
        }

        Ok(ComponentInspection {
            imports,
            exports,
            component_sha256: sha256_hex(raw_bytes),
        })
    }
}

/// Encodes a WAT core module against the `workflow-validator` world in `WitContract`
/// into a validated WebAssembly Component Model binary.
///
/// # Errors
/// Returns [`WasmError::ComponentBuild`] if WAT assembly, WIT metadata embedding,
/// or Component Model encoding/validation fails.
pub fn compile_wat_component(
    wat_source: &str,
    contract: &WitContract,
) -> Result<Vec<u8>, WasmError> {
    let mut core_wasm = wat::parse_str(wat_source)
        .map_err(|err| WasmError::ComponentBuild(format!("WAT parse failed: {err}")))?;

    let world_id = contract
        .resolve()
        .select_world(&[contract.package_id()], Some(EXPECTED_VALIDATOR_WORLD))
        .map_err(|err| WasmError::ComponentBuild(format!("WIT world lookup failed: {err}")))?;

    wit_component::embed_component_metadata(
        &mut core_wasm,
        contract.resolve(),
        world_id,
        wit_component::StringEncoding::UTF8,
        false, // canonical_names: keep exact versioned names, as before 0.263.
    )
    .map_err(|err| WasmError::ComponentBuild(format!("metadata embed failed: {err}")))?;

    wit_component::ComponentEncoder::default()
        .module(&core_wasm)
        .map_err(|err| WasmError::ComponentBuild(format!("component module load failed: {err}")))?
        .validate(true)
        .encode()
        .map_err(|err| WasmError::ComponentBuild(format!("component encode failed: {err}")))
}

fn map_wasmtime_error(context: &str, err: &wasmtime::Error) -> WasmError {
    if let Some(trap) = err.downcast_ref::<Trap>() {
        return WasmError::Execution(format!("{context}: trapped ({trap:?})"));
    }
    WasmError::Execution(format!("{context}: {err}"))
}

/// Helper to build default bounded resource limits.
#[must_use]
pub fn default_limits() -> ResourceLimits {
    ResourceLimits::default()
}
