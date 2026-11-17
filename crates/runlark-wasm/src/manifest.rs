use crate::error::WasmError;
use crate::wit_contract::{EXPECTED_VALIDATOR_WORLD, EXPECTED_WIT_PACKAGE, WitContract};
use crate::workflow::WorkflowDefinition;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Hard host ceiling on Wasmtime execution fuel per validation call.
pub const HOST_MAX_FUEL: u64 = 10_000_000;
/// Hard host ceiling on linear memory bytes per validator instance (64 MiB).
pub const HOST_MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;
/// Hard host ceiling on WASM tables per validator instance.
pub const HOST_MAX_TABLES: usize = 16;
/// Hard host ceiling on component instances per store.
pub const HOST_MAX_INSTANCES: usize = 16;

/// Canonical SHA-256 package digest (64 lowercase hexadecimal characters).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PackageDigest(pub String);

impl PackageDigest {
    /// Returns the digest string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackageDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for PackageDigest {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Kind of versioned Runlark extension package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageKind {
    /// Full declarative workflow plus its WASM validator component.
    Workflow,
    /// Standalone WASM validator component.
    Validator,
}

impl PackageKind {
    /// Canonical lowercase string form used in manifest digest calculation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Workflow => "workflow",
            Self::Validator => "validator",
        }
    }
}

/// Resource ceilings declared by a package and clamped to host maximums.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub max_fuel: u64,
    pub max_memory_bytes: usize,
    pub max_tables: usize,
    pub max_instances: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_fuel: 1_000_000,
            max_memory_bytes: 16 * 1024 * 1024,
            max_tables: 4,
            max_instances: 4,
        }
    }
}

impl ResourceLimits {
    /// Clamps requested package limits to the host's hard maximum bounds.
    #[must_use]
    pub fn clamped(self) -> Self {
        Self {
            max_fuel: self.max_fuel.min(HOST_MAX_FUEL),
            max_memory_bytes: self.max_memory_bytes.min(HOST_MAX_MEMORY_BYTES),
            max_tables: self.max_tables.min(HOST_MAX_TABLES),
            max_instances: self.max_instances.min(HOST_MAX_INSTANCES),
        }
    }
}

/// Versioned package manifest (`manifest.json`) for workflows and validators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageManifest {
    pub schema_version: u32,
    pub name: String,
    pub version: String,
    pub kind: PackageKind,
    pub wit_version: String,
    pub world: String,
    pub entrypoint: String,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_sha256: Option<String>,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub limits: ResourceLimits,
}

/// Verified package artifacts with validated manifest, digests, and component bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPackage {
    pub manifest: PackageManifest,
    pub package_digest: PackageDigest,
    pub component_bytes: Vec<u8>,
    pub workflow: Option<WorkflowDefinition>,
    pub workflow_bytes: Option<Vec<u8>>,
}

/// Computes lowercase hex SHA-256 digest of a byte slice.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

impl PackageManifest {
    /// Parses `manifest.json` bytes and validates all fields against the WIT contract.
    ///
    /// # Errors
    /// Returns [`WasmError::Json`] or [`WasmError::InvalidManifest`] if parsing or validation fails.
    pub fn from_json_bytes(bytes: &[u8], contract: &WitContract) -> Result<Self, WasmError> {
        let manifest: Self = serde_json::from_slice(bytes)?;
        manifest.validate(contract)?;
        Ok(manifest)
    }

    /// Validates manifest schema, `SemVer`, WIT contract version, relative paths, SHA-256 format,
    /// capability allowlist, and resource bounds.
    ///
    /// # Errors
    /// Returns [`WasmError::InvalidManifest`] if any invariant is violated.
    pub fn validate(&self, contract: &WitContract) -> Result<(), WasmError> {
        if self.schema_version != 1 {
            return Err(WasmError::InvalidManifest(format!(
                "unsupported schema_version {}, expected 1",
                self.schema_version
            )));
        }
        validate_package_name(&self.name)?;
        validate_semver(&self.version)?;
        if self.wit_version != EXPECTED_WIT_PACKAGE {
            return Err(WasmError::InvalidManifest(format!(
                "unsupported wit_version `{}`, expected `{EXPECTED_WIT_PACKAGE}`",
                self.wit_version
            )));
        }
        if self.world != EXPECTED_VALIDATOR_WORLD {
            return Err(WasmError::InvalidManifest(format!(
                "unsupported world `{}`, expected `{EXPECTED_VALIDATOR_WORLD}`",
                self.world
            )));
        }
        validate_relative_path(&self.entrypoint, "entrypoint")?;
        validate_sha256_hex(&self.sha256, "sha256")?;

        match self.kind {
            PackageKind::Workflow => {
                let wf_path = self.workflow_path.as_deref().ok_or_else(|| {
                    WasmError::InvalidManifest(
                        "workflow package must declare `workflow_path`".to_string(),
                    )
                })?;
                let wf_sha = self.workflow_sha256.as_deref().ok_or_else(|| {
                    WasmError::InvalidManifest(
                        "workflow package must declare `workflow_sha256`".to_string(),
                    )
                })?;
                validate_relative_path(wf_path, "workflow_path")?;
                validate_sha256_hex(wf_sha, "workflow_sha256")?;
            }
            PackageKind::Validator => {
                if self.workflow_path.is_some() || self.workflow_sha256.is_some() {
                    return Err(WasmError::InvalidManifest(
                        "validator-only package must not declare workflow_path/workflow_sha256"
                            .to_string(),
                    ));
                }
            }
        }

        let allowed_caps = contract.allowed_capabilities();
        let mut seen = BTreeSet::new();
        for cap in &self.capabilities {
            if !allowed_caps.contains(cap) {
                return Err(WasmError::InvalidManifest(format!(
                    "disallowed capability `{cap}` not declared in WIT contract"
                )));
            }
            if !seen.insert(cap.as_str()) {
                return Err(WasmError::InvalidManifest(format!(
                    "duplicate capability `{cap}` in manifest"
                )));
            }
        }

        if self.limits.max_fuel == 0
            || self.limits.max_memory_bytes == 0
            || self.limits.max_tables == 0
            || self.limits.max_instances == 0
        {
            return Err(WasmError::InvalidManifest(
                "resource limits (fuel, memory, tables, instances) must be positive".to_string(),
            ));
        }

        Ok(())
    }

    /// Computes the canonical package digest pinning manifest metadata and artifact hashes.
    #[must_use]
    pub fn canonical_digest(&self) -> PackageDigest {
        let mut sorted_caps = self.capabilities.clone();
        sorted_caps.sort();
        let wf_sha = self.workflow_sha256.as_deref().unwrap_or("");
        let canonical = format!(
            "v{}|{}|{}|{}|{}|{}|{}|{}|{}|{}:{}:{}:{}",
            self.schema_version,
            self.name,
            self.version,
            self.kind.as_str(),
            self.wit_version,
            self.world,
            self.sha256,
            wf_sha,
            sorted_caps.join(","),
            self.limits.max_fuel,
            self.limits.max_memory_bytes,
            self.limits.max_tables,
            self.limits.max_instances,
        );
        PackageDigest(sha256_hex(canonical.as_bytes()))
    }
}

impl VerifiedPackage {
    /// Loads and cryptographically verifies a package directory (or `manifest.json` file path)
    /// against the canonical WIT contract.
    ///
    /// # Errors
    /// Returns [`WasmError`] if reading files, parsing manifest/workflow, or SHA-256 verification fails.
    pub fn load_from_path(path: &Path, contract: &WitContract) -> Result<Self, WasmError> {
        let (pkg_dir, manifest_path) = if path.is_file() {
            let parent = path.parent().ok_or_else(|| {
                WasmError::InvalidManifest("manifest path has no parent directory".to_string())
            })?;
            (parent.to_path_buf(), path.to_path_buf())
        } else {
            (path.to_path_buf(), path.join("manifest.json"))
        };

        let manifest_bytes = std::fs::read(&manifest_path)?;
        let manifest = PackageManifest::from_json_bytes(&manifest_bytes, contract)?;

        let component_rel = validate_relative_path(&manifest.entrypoint, "entrypoint")?;
        let component_path = pkg_dir.join(component_rel);
        let component_bytes = std::fs::read(&component_path)?;
        let actual_component_sha = sha256_hex(&component_bytes);
        if actual_component_sha != manifest.sha256 {
            return Err(WasmError::DigestMismatch {
                artifact: manifest.entrypoint.clone(),
                expected: manifest.sha256.clone(),
                actual: actual_component_sha,
            });
        }

        let (workflow, workflow_bytes) = if let (Some(wf_path_str), Some(wf_sha)) = (
            manifest.workflow_path.as_deref(),
            manifest.workflow_sha256.as_deref(),
        ) {
            let wf_rel = validate_relative_path(wf_path_str, "workflow_path")?;
            let wf_bytes = std::fs::read(pkg_dir.join(wf_rel))?;
            let actual_wf_sha = sha256_hex(&wf_bytes);
            if actual_wf_sha != wf_sha {
                return Err(WasmError::DigestMismatch {
                    artifact: wf_path_str.to_string(),
                    expected: wf_sha.to_string(),
                    actual: actual_wf_sha,
                });
            }
            let def: WorkflowDefinition = serde_json::from_slice(&wf_bytes)?;
            def.validate(contract)?;
            (Some(def), Some(wf_bytes))
        } else {
            (None, None)
        };

        let package_digest = manifest.canonical_digest();
        Ok(Self {
            manifest,
            package_digest,
            component_bytes,
            workflow,
            workflow_bytes,
        })
    }

    /// Re-verifies component and workflow SHA-256 digests in memory.
    ///
    /// # Errors
    /// Returns [`WasmError::DigestMismatch`] if any artifact bytes no longer match the manifest.
    pub fn verify_digests(&self) -> Result<(), WasmError> {
        let actual_component_sha = sha256_hex(&self.component_bytes);
        if actual_component_sha != self.manifest.sha256 {
            return Err(WasmError::DigestMismatch {
                artifact: self.manifest.entrypoint.clone(),
                expected: self.manifest.sha256.clone(),
                actual: actual_component_sha,
            });
        }
        if let (Some(expected_wf), Some(wf_bytes)) = (
            self.manifest.workflow_sha256.as_deref(),
            self.workflow_bytes.as_deref(),
        ) {
            let actual_wf_sha = sha256_hex(wf_bytes);
            if actual_wf_sha != expected_wf {
                return Err(WasmError::DigestMismatch {
                    artifact: self
                        .manifest
                        .workflow_path
                        .clone()
                        .unwrap_or_else(|| "workflow.json".to_string()),
                    expected: expected_wf.to_string(),
                    actual: actual_wf_sha,
                });
            }
        }
        Ok(())
    }
}

fn validate_package_name(name: &str) -> Result<(), WasmError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed != name || trimmed.contains("..") {
        return Err(WasmError::InvalidManifest(format!(
            "invalid package name `{name}`"
        )));
    }
    let valid = trimmed
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.'));
    if !valid {
        return Err(WasmError::InvalidManifest(format!(
            "package name `{name}` must match [a-z0-9._-]+"
        )));
    }
    Ok(())
}

fn validate_semver(version: &str) -> Result<(), WasmError> {
    let core = version.split(['-', '+']).next().unwrap_or("");
    let mut parts = core.split('.');
    let Some(major) = parts.next() else {
        return Err(WasmError::InvalidManifest(format!(
            "invalid semver `{version}`"
        )));
    };
    let Some(minor) = parts.next() else {
        return Err(WasmError::InvalidManifest(format!(
            "invalid semver `{version}`"
        )));
    };
    let Some(patch) = parts.next() else {
        return Err(WasmError::InvalidManifest(format!(
            "invalid semver `{version}`"
        )));
    };
    if parts.next().is_some()
        || [major, minor, patch]
            .iter()
            .any(|seg| seg.is_empty() || !seg.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(WasmError::InvalidManifest(format!(
            "invalid semver `{version}`"
        )));
    }
    Ok(())
}

fn validate_relative_path(raw: &str, field: &str) -> Result<PathBuf, WasmError> {
    if raw.is_empty() || raw.contains('\\') {
        return Err(WasmError::InvalidManifest(format!(
            "{field} `{raw}` must be a non-empty forward-slash relative path"
        )));
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(WasmError::InvalidManifest(format!(
            "{field} `{raw}` must not be an absolute path"
        )));
    }
    for comp in path.components() {
        if !matches!(comp, Component::Normal(_)) {
            return Err(WasmError::InvalidManifest(format!(
                "{field} `{raw}` must not contain `.` or `..` traversal"
            )));
        }
    }
    Ok(path.to_path_buf())
}

fn validate_sha256_hex(digest: &str, field: &str) -> Result<(), WasmError> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    {
        return Err(WasmError::InvalidManifest(format!(
            "{field} must be 64 lowercase hexadecimal characters"
        )));
    }
    Ok(())
}
