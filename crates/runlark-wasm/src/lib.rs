pub mod error;
pub mod manifest;
pub mod registry;
pub mod validator_host;
pub mod wit_contract;
pub mod workflow;

pub use error::WasmError;
pub use manifest::{
    HOST_MAX_FUEL, HOST_MAX_INSTANCES, HOST_MAX_MEMORY_BYTES, HOST_MAX_TABLES, PackageDigest,
    PackageKind, PackageManifest, ResourceLimits, VerifiedPackage, sha256_hex,
};
pub use registry::{DigestPinnedRegistry, PackagePin};
pub use validator_host::{
    CheckEvidence, ComponentInspection, HostLogEntry, ValidationInput, ValidationReport,
    ValidationVerdict, WasmValidatorHost, WorkflowStage, compile_wat_component,
};
pub use wit_contract::{
    CANONICAL_WIT_SOURCE, EXPECTED_VALIDATOR_INTERFACE, EXPECTED_VALIDATOR_WORLD,
    EXPECTED_WIT_PACKAGE, WitContract, WitContractInfo,
};
pub use workflow::{StageTransition, WorkflowBudget, WorkflowDefinition};

use serde::Serialize;
use std::path::Path;

/// Full inspection output for a verified Runlark workflow or validator package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageInspection {
    pub manifest: PackageManifest,
    pub package_digest: PackageDigest,
    pub wit_contract: WitContractInfo,
    pub component: ComponentInspection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow: Option<WorkflowDefinition>,
}

/// Loads, cryptographically verifies, and inspects a package directory or `manifest.json` path
/// against the canonical WIT contract and Wasmtime component type checker.
///
/// # Errors
/// Returns [`WasmError`] if the WIT contract, manifest, SHA-256 digests, workflow definition,
/// or WASM component imports/exports are invalid.
pub fn inspect_package_path(path: &Path) -> Result<PackageInspection, WasmError> {
    let contract = WitContract::canonical()?;
    let package = VerifiedPackage::load_from_path(path, &contract)?;
    let host = WasmValidatorHost::new()?;
    let component =
        host.inspect_component(&package.component_bytes, &package.manifest.capabilities)?;
    Ok(PackageInspection {
        manifest: package.manifest,
        package_digest: package.package_digest,
        wit_contract: contract.info().clone(),
        component,
        workflow: package.workflow,
    })
}

/// Validates a package directory or `manifest.json` path end-to-end, including a bounded
/// Wasmtime smoke invocation of `validate-run` to prove runtime execution works.
///
/// # Errors
/// Returns [`WasmError`] if verification or bounded Wasmtime validator execution fails.
pub fn validate_package_path(
    path: &Path,
) -> Result<(PackageInspection, ValidationReport), WasmError> {
    let contract = WitContract::canonical()?;
    let package = VerifiedPackage::load_from_path(path, &contract)?;
    let host = WasmValidatorHost::new()?;
    let component =
        host.inspect_component(&package.component_bytes, &package.manifest.capabilities)?;
    let probe_input = ValidationInput {
        task_id: "probe-task".to_string(),
        run_id: "probe-run".to_string(),
        stage: WorkflowStage::Validate,
        changed_files: vec!["src/lib.rs".to_string()],
        checks: vec![CheckEvidence {
            name: "probe-check".to_string(),
            passed: true,
            exit_code: 0,
        }],
        diff_bytes: 128,
        repair_attempt: 0,
    };
    let report = host.validate_run(&package, &probe_input)?;
    let inspection = PackageInspection {
        manifest: package.manifest,
        package_digest: package.package_digest,
        wit_contract: contract.info().clone(),
        component,
        workflow: package.workflow,
    };
    Ok((inspection, report))
}
