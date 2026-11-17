use runlark_domain::{ArtifactDigest, RunId};
use runlark_wasm::{
    DigestPinnedRegistry, PackageDigest, PackagePin, ValidationInput, ValidationReport,
    VerifiedPackage, WasmError, WasmValidatorHost, WitContract,
};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Coordinates digest-pinned workflow packages and bounded Wasmtime run verification.
#[derive(Clone)]
pub struct VerificationService {
    host: Arc<WasmValidatorHost>,
    registry: Arc<Mutex<DigestPinnedRegistry>>,
    contract: Arc<WitContract>,
}

impl VerificationService {
    /// Creates an in-memory verification service (used for standalone tests or ephemeral state).
    ///
    /// # Errors
    /// Returns [`WasmError`] if the canonical WIT contract or Wasmtime engine fails to initialize.
    pub fn in_memory() -> Result<Self, WasmError> {
        Ok(Self {
            host: Arc::new(WasmValidatorHost::new()?),
            registry: Arc::new(Mutex::new(DigestPinnedRegistry::new())),
            contract: Arc::new(WitContract::canonical()?),
        })
    }

    /// Creates a verification service backed by a content-addressed directory on disk.
    ///
    /// # Errors
    /// Returns [`WasmError`] if CAS directory initialization, WIT parsing, or Wasmtime init fails.
    pub fn with_cas_dir(cas_dir: impl AsRef<Path>) -> Result<Self, WasmError> {
        Ok(Self {
            host: Arc::new(WasmValidatorHost::new()?),
            registry: Arc::new(Mutex::new(DigestPinnedRegistry::with_cas_dir(
                cas_dir.as_ref(),
            )?)),
            contract: Arc::new(WitContract::canonical()?),
        })
    }

    /// Installs and cryptographically verifies a workflow/validator package from `path`.
    ///
    /// # Errors
    /// Returns [`WasmError`] if loading, verification, or component inspection fails.
    pub fn install_package_dir(&self, path: &Path) -> Result<PackagePin, WasmError> {
        let pkg = VerifiedPackage::load_from_path(path, &self.contract)?;
        self.host
            .inspect_component(&pkg.component_bytes, &pkg.manifest.capabilities)?;
        let mut guard = self
            .registry
            .lock()
            .map_err(|_| WasmError::Execution("registry lock poisoned".to_string()))?;
        guard.install_package(pkg)
    }

    /// Lists all package pins currently retained in the registry.
    ///
    /// # Errors
    /// Returns [`WasmError::Execution`] if the registry mutex is poisoned.
    pub fn list_packages(&self) -> Result<Vec<PackagePin>, WasmError> {
        let guard = self
            .registry
            .lock()
            .map_err(|_| WasmError::Execution("registry lock poisoned".to_string()))?;
        let mut pins = Vec::new();
        for digest in guard.retained_digests() {
            if let Some(pkg) = guard.get_by_digest(&digest) {
                pins.push(PackagePin::from(pkg));
            }
        }
        Ok(pins)
    }

    /// Returns the latest installed [`ArtifactDigest`] for `package_name`, if present.
    ///
    /// # Errors
    /// Returns [`WasmError::Execution`] if the registry mutex is poisoned.
    pub fn latest_digest(&self, package_name: &str) -> Result<Option<ArtifactDigest>, WasmError> {
        let guard = self
            .registry
            .lock()
            .map_err(|_| WasmError::Execution("registry lock poisoned".to_string()))?;
        let Some(pkg) = guard.latest_for(package_name) else {
            return Ok(None);
        };
        ArtifactDigest::parse(pkg.package_digest.as_str())
            .map(Some)
            .map_err(|err| WasmError::Execution(err.to_string()))
    }

    /// Pins (if not already pinned) and executes the WASM validator for `run_id` using
    /// the digest-pinned package and bounded Wasmtime resource limits.
    ///
    /// # Errors
    /// Returns [`WasmError`] if the package is missing or validation/execution fails.
    pub fn verify_run(
        &self,
        run_id: &RunId,
        package_name: &str,
        input: &ValidationInput,
    ) -> Result<ValidationReport, WasmError> {
        self.verify_run_with_digest(run_id, None, package_name, input)
    }

    /// Executes the WASM validator for `run_id`, preferring an explicit pinned `digest`
    /// (loaded from memory or CAS) before falling back to the latest version of `package_name`.
    ///
    /// # Errors
    /// Returns [`WasmError`] if the package is missing, digest verification fails, or execution traps.
    pub fn verify_run_with_digest(
        &self,
        run_id: &RunId,
        digest: Option<&ArtifactDigest>,
        package_name: &str,
        input: &ValidationInput,
    ) -> Result<ValidationReport, WasmError> {
        let package = {
            let mut guard = self
                .registry
                .lock()
                .map_err(|_| WasmError::Execution("registry lock poisoned".to_string()))?;
            if let Some(pinned) = guard.get_for_run(run_id) {
                pinned.clone()
            } else if let Some(explicit_digest) = digest {
                let pkg_digest = PackageDigest(explicit_digest.as_str().to_string());
                let loaded = guard.load_by_digest(&pkg_digest, &self.contract)?.clone();
                guard.pin_run_to_digest(run_id, &pkg_digest)?;
                loaded
            } else {
                guard.pin_run(run_id, package_name)?;
                guard
                    .get_for_run(run_id)
                    .ok_or_else(|| WasmError::PackageNotFound(package_name.to_string()))?
                    .clone()
            }
        };
        self.host.validate_run(&package, input)
    }

    /// Releases a run's digest pin once the run has terminated.
    ///
    /// # Errors
    /// Returns [`WasmError::Execution`] if the registry mutex is poisoned.
    pub fn release_run(&self, run_id: &RunId) -> Result<(), WasmError> {
        let mut guard = self
            .registry
            .lock()
            .map_err(|_| WasmError::Execution("registry lock poisoned".to_string()))?;
        guard.release_run(run_id);
        Ok(())
    }
}
