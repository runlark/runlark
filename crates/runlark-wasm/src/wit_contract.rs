use crate::error::WasmError;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;
use wit_parser::{PackageId, Resolve, TypeDefKind, UnresolvedPackageGroup};

/// Embedded canonical `runlark:workflow@0.1.0` WIT contract source.
pub const CANONICAL_WIT_SOURCE: &str = include_str!("../../../wit/runlark-workflow.wit");

/// Expected WIT package identifier for A1 workflow/validator packages.
pub const EXPECTED_WIT_PACKAGE: &str = "runlark:workflow@0.1.0";

/// Expected WIT world for validator components.
pub const EXPECTED_VALIDATOR_WORLD: &str = "workflow-validator";

/// Expected WIT shared types interface identifier.
pub const EXPECTED_TYPES_INTERFACE: &str = "runlark:workflow/types@0.1.0";

/// Expected WIT validator interface identifier.
pub const EXPECTED_VALIDATOR_INTERFACE: &str = "runlark:workflow/validator@0.1.0";

/// Parsed summary of the versioned WIT contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WitContractInfo {
    pub package_name: String,
    pub interfaces: Vec<String>,
    pub worlds: Vec<String>,
    pub stages: Vec<String>,
    pub capabilities: Vec<String>,
}

/// Validated `wit_parser::Resolve` wrapper for the Runlark workflow WIT contract.
#[derive(Debug, Clone)]
pub struct WitContract {
    resolve: Resolve,
    package_id: PackageId,
    info: WitContractInfo,
}

impl WitContract {
    /// Parses and validates the embedded canonical `wit/runlark-workflow.wit` contract.
    ///
    /// # Errors
    /// Returns [`WasmError::WitContract`] if the WIT source fails to parse or violates
    /// expected package, interface, world, stage, or capability requirements.
    pub fn canonical() -> Result<Self, WasmError> {
        Self::parse_str("wit/runlark-workflow.wit", CANONICAL_WIT_SOURCE)
    }

    /// Parses and validates a WIT file from disk against Runlark's required contract shape.
    ///
    /// # Errors
    /// Returns [`WasmError::WitContract`] if reading, parsing, or validating fails.
    pub fn from_file(path: &Path) -> Result<Self, WasmError> {
        let source = std::fs::read_to_string(path).map_err(|err| {
            WasmError::WitContract(format!("failed to read {}: {err}", path.display()))
        })?;
        let label = path.display().to_string();
        Self::parse_str(&label, &source)
    }

    /// Parses and validates WIT source text against Runlark's required contract shape.
    ///
    /// # Errors
    /// Returns [`WasmError::WitContract`] if parsing fails or required interfaces, worlds,
    /// or enum cases are missing.
    pub fn parse_str(path_label: &str, source: &str) -> Result<Self, WasmError> {
        let mut resolve = Resolve::default();
        // wit-parser errors carry spans; render them to keep file:line:col context.
        let group = UnresolvedPackageGroup::parse(path_label, source).map_err(|(map, err)| {
            WasmError::WitContract(format!("WIT parse failed: {}", err.render(&map)))
        })?;
        let package_id = resolve.push_group(group).map_err(|err| {
            let rendered = err.render(&resolve.source_map);
            WasmError::WitContract(format!("WIT resolve failed: {rendered}"))
        })?;

        let pkg = resolve
            .packages
            .get(package_id)
            .ok_or_else(|| WasmError::WitContract("resolved package missing".to_string()))?;
        let package_name = pkg.name.to_string();
        if package_name != EXPECTED_WIT_PACKAGE {
            return Err(WasmError::WitContract(format!(
                "unexpected WIT package `{package_name}`, expected `{EXPECTED_WIT_PACKAGE}`"
            )));
        }

        let interfaces: Vec<String> = pkg.interfaces.keys().cloned().collect();
        for required in ["types", "planning", "validator"] {
            if !pkg.interfaces.contains_key(required) {
                return Err(WasmError::WitContract(format!(
                    "missing required WIT interface `{required}`"
                )));
            }
        }

        let worlds: Vec<String> = pkg.worlds.keys().cloned().collect();
        if !pkg.worlds.contains_key(EXPECTED_VALIDATOR_WORLD) {
            return Err(WasmError::WitContract(format!(
                "missing required WIT world `{EXPECTED_VALIDATOR_WORLD}`"
            )));
        }

        let types_iface_id = pkg.interfaces["types"];
        let types_iface = &resolve.interfaces[types_iface_id];
        let stages = extract_enum_cases(&resolve, types_iface, "workflow-stage")?;
        let capabilities = extract_enum_cases(&resolve, types_iface, "capability")?;

        let info = WitContractInfo {
            package_name,
            interfaces,
            worlds,
            stages,
            capabilities,
        };
        Ok(Self {
            resolve,
            package_id,
            info,
        })
    }

    /// Returns the parsed metadata summary for this WIT contract.
    #[must_use]
    pub fn info(&self) -> &WitContractInfo {
        &self.info
    }

    /// Returns the underlying `wit_parser::Resolve`.
    #[must_use]
    pub fn resolve(&self) -> &Resolve {
        &self.resolve
    }

    /// Returns the resolved [`PackageId`] for `runlark:workflow@0.1.0`.
    #[must_use]
    pub fn package_id(&self) -> PackageId {
        self.package_id
    }

    /// Returns the set of capability names allowed by the WIT `capability` enum.
    #[must_use]
    pub fn allowed_capabilities(&self) -> BTreeSet<String> {
        self.info.capabilities.iter().cloned().collect()
    }

    /// Returns the set of workflow stage names allowed by the WIT `workflow-stage` enum.
    #[must_use]
    pub fn allowed_stages(&self) -> BTreeSet<String> {
        self.info.stages.iter().cloned().collect()
    }
}

fn extract_enum_cases(
    resolve: &Resolve,
    iface: &wit_parser::Interface,
    enum_name: &str,
) -> Result<Vec<String>, WasmError> {
    let type_id = iface
        .types
        .get(enum_name)
        .copied()
        .ok_or_else(|| WasmError::WitContract(format!("missing WIT enum `{enum_name}`")))?;
    let type_def = resolve
        .types
        .get(type_id)
        .ok_or_else(|| WasmError::WitContract(format!("WIT type `{enum_name}` missing")))?;
    match &type_def.kind {
        TypeDefKind::Enum(enum_def) => Ok(enum_def.cases.iter().map(|c| c.name.clone()).collect()),
        _ => Err(WasmError::WitContract(format!(
            "WIT type `{enum_name}` must be an enum"
        ))),
    }
}
