use runlark_wasm::{PackageInspection, ValidationReport};
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
struct ValidateOutput {
    inspection: PackageInspection,
    validation: ValidationReport,
}

/// Validates a Runlark workflow/validator package directory or `manifest.json` file,
/// verifying WIT compatibility, SHA-256 digests, workflow rules, and bounded Wasmtime execution.
///
/// # Errors
/// Returns an error if verification, component inspection, or Wasmtime execution fails.
pub fn run_plugin_validate(
    path: &Path,
    json: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (inspection, validation) = runlark_wasm::validate_package_path(path)?;
    if json {
        let out = ValidateOutput {
            inspection,
            validation,
        };
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!(
            "VALID {}@{} (digest: {})",
            inspection.manifest.name, inspection.manifest.version, inspection.package_digest
        );
        println!("  wit: {}", inspection.wit_contract.package_name);
        println!(
            "  component sha256: {}",
            inspection.component.component_sha256
        );
        println!(
            "  probe verdict: {:?} ({}, fuel_used={}/{})",
            validation.verdict, validation.summary, validation.fuel_consumed, validation.fuel_limit
        );
    }
    Ok(())
}

/// Inspects a Runlark workflow/validator package directory or `manifest.json` file,
/// reporting manifest metadata, digests, WIT interfaces, capabilities, and component exports.
///
/// # Errors
/// Returns an error if loading or verifying the package fails.
pub fn run_plugin_inspect(
    path: &Path,
    json: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let inspection = runlark_wasm::inspect_package_path(path)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&inspection)?);
    } else {
        println!(
            "{}@{} ({:?})",
            inspection.manifest.name, inspection.manifest.version, inspection.manifest.kind
        );
        println!("  package_digest: {}", inspection.package_digest);
        println!("  wit_version: {}", inspection.manifest.wit_version);
        println!("  world: {}", inspection.manifest.world);
        println!(
            "  capabilities: {}",
            inspection.manifest.capabilities.join(", ")
        );
        println!("  imports: {}", inspection.component.imports.join(", "));
        println!("  exports: {}", inspection.component.exports.join(", "));
        if let Some(workflow) = &inspection.workflow {
            println!("  stages: {}", workflow.stages.join(" -> "));
        }
    }
    Ok(())
}
