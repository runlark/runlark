use crate::error::WasmError;
use crate::manifest::{PackageDigest, VerifiedPackage};
use crate::wit_contract::WitContract;
use runlark_domain::RunId;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// Summary of a package digest pinned for a run or installed in the registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackagePin {
    pub name: String,
    pub version: String,
    pub package_digest: PackageDigest,
    pub component_sha256: String,
}

impl From<&VerifiedPackage> for PackagePin {
    fn from(pkg: &VerifiedPackage) -> Self {
        Self {
            name: pkg.manifest.name.clone(),
            version: pkg.manifest.version.clone(),
            package_digest: pkg.package_digest.clone(),
            component_sha256: pkg.manifest.sha256.clone(),
        }
    }
}

/// Content-addressed, digest-pinned workflow and validator package store.
///
/// Installing a newer version updates the latest pointer for new runs while retaining
/// previously pinned digests for all in-flight runs until explicitly released.
#[derive(Debug, Default)]
pub struct DigestPinnedRegistry {
    cas_root: Option<PathBuf>,
    packages_by_digest: BTreeMap<PackageDigest, VerifiedPackage>,
    latest_by_name: BTreeMap<String, PackageDigest>,
    run_pins: HashMap<RunId, PackageDigest>,
}

impl DigestPinnedRegistry {
    /// Creates an in-memory digest-pinned registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a digest-pinned registry backed by a content-addressed directory on disk.
    ///
    /// # Errors
    /// Returns [`WasmError::Io`] if creating the CAS directory fails.
    pub fn with_cas_dir(cas_root: impl Into<PathBuf>) -> Result<Self, WasmError> {
        let root = cas_root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Self {
            cas_root: Some(root),
            ..Self::default()
        })
    }

    /// Loads, verifies, and installs a package directory (or manifest path) into the registry.
    ///
    /// # Errors
    /// Returns [`WasmError`] if manifest parsing, digest verification, or CAS write fails.
    pub fn install_from_path(
        &mut self,
        path: &Path,
        contract: &WitContract,
    ) -> Result<PackagePin, WasmError> {
        let pkg = VerifiedPackage::load_from_path(path, contract)?;
        self.install_package(pkg)
    }

    /// Installs an already-verified package, updating the latest digest for its name
    /// without evicting older digests.
    ///
    /// # Errors
    /// Returns [`WasmError`] if digest re-verification or CAS persistence fails.
    pub fn install_package(&mut self, package: VerifiedPackage) -> Result<PackagePin, WasmError> {
        package.verify_digests()?;
        let pin = PackagePin::from(&package);
        let digest = package.package_digest.clone();
        let name = package.manifest.name.clone();

        if let Some(root) = &self.cas_root {
            persist_to_cas(root, &package)?;
        }

        self.packages_by_digest.insert(digest.clone(), package);
        self.latest_by_name.insert(name, digest);
        Ok(pin)
    }

    /// Returns the latest verified package installed for `name`, if any.
    #[must_use]
    pub fn latest_for(&self, name: &str) -> Option<&VerifiedPackage> {
        let digest = self.latest_by_name.get(name)?;
        self.packages_by_digest.get(digest)
    }

    /// Returns a verified package by its exact [`PackageDigest`] from memory.
    #[must_use]
    pub fn get_by_digest(&self, digest: &PackageDigest) -> Option<&VerifiedPackage> {
        self.packages_by_digest.get(digest)
    }

    /// Resolves a verified package by [`PackageDigest`], falling back to the CAS directory
    /// if configured and re-verifying SHA-256 digests on load.
    ///
    /// # Errors
    /// Returns [`WasmError::PackageNotFound`] if missing, or [`WasmError`] if CAS verification fails.
    pub fn load_by_digest(
        &mut self,
        digest: &PackageDigest,
        contract: &WitContract,
    ) -> Result<&VerifiedPackage, WasmError> {
        if !self.packages_by_digest.contains_key(digest) {
            let Some(root) = &self.cas_root else {
                return Err(WasmError::PackageNotFound(digest.0.clone()));
            };
            let blob_dir = root.join(&digest.0);
            if !blob_dir.exists() {
                return Err(WasmError::PackageNotFound(digest.0.clone()));
            }
            let loaded = VerifiedPackage::load_from_path(&blob_dir, contract)?;
            if &loaded.package_digest != digest {
                return Err(WasmError::DigestMismatch {
                    artifact: format!("cas/{}", digest.0),
                    expected: digest.0.clone(),
                    actual: loaded.package_digest.0,
                });
            }
            self.packages_by_digest.insert(digest.clone(), loaded);
        }
        self.packages_by_digest
            .get(digest)
            .ok_or_else(|| WasmError::PackageNotFound(digest.0.clone()))
    }

    /// Pins an in-flight run to the current latest digest installed for `package_name`.
    ///
    /// # Errors
    /// Returns [`WasmError::PackageNotFound`] if `package_name` has no installed version.
    pub fn pin_run(&mut self, run_id: &RunId, package_name: &str) -> Result<PackagePin, WasmError> {
        let digest = self
            .latest_by_name
            .get(package_name)
            .cloned()
            .ok_or_else(|| WasmError::PackageNotFound(package_name.to_string()))?;
        self.pin_run_to_digest(run_id, &digest)
    }

    /// Pins an in-flight run to an explicit [`PackageDigest`].
    ///
    /// # Errors
    /// Returns [`WasmError::PackageNotFound`] if the digest is not installed.
    pub fn pin_run_to_digest(
        &mut self,
        run_id: &RunId,
        digest: &PackageDigest,
    ) -> Result<PackagePin, WasmError> {
        let pkg = self
            .packages_by_digest
            .get(digest)
            .ok_or_else(|| WasmError::PackageNotFound(digest.0.clone()))?;
        let pin = PackagePin::from(pkg);
        self.run_pins.insert(run_id.clone(), digest.clone());
        Ok(pin)
    }

    /// Returns the verified package pinned to `run_id`, if pinned.
    #[must_use]
    pub fn get_for_run(&self, run_id: &RunId) -> Option<&VerifiedPackage> {
        let digest = self.run_pins.get(run_id)?;
        self.packages_by_digest.get(digest)
    }

    /// Releases the digest pin held by a completed/stopped run.
    pub fn release_run(&mut self, run_id: &RunId) -> Option<PackageDigest> {
        self.run_pins.remove(run_id)
    }

    /// Returns all currently retained package digests in memory.
    #[must_use]
    pub fn retained_digests(&self) -> Vec<PackageDigest> {
        self.packages_by_digest.keys().cloned().collect()
    }

    /// Removes digests that are neither the latest version for a package name
    /// nor pinned by any in-flight run. Returns the number of evicted packages.
    pub fn gc_unreferenced(&mut self) -> usize {
        let mut keep: BTreeSet<PackageDigest> = self.latest_by_name.values().cloned().collect();
        keep.extend(self.run_pins.values().cloned());

        let to_remove: Vec<PackageDigest> = self
            .packages_by_digest
            .keys()
            .filter(|digest| !keep.contains(*digest))
            .cloned()
            .collect();

        let count = to_remove.len();
        for digest in to_remove {
            self.packages_by_digest.remove(&digest);
            if let Some(root) = &self.cas_root {
                let blob_dir = root.join(&digest.0);
                if blob_dir.exists() {
                    let _ = std::fs::remove_dir_all(blob_dir);
                }
            }
        }
        count
    }
}

fn persist_to_cas(root: &Path, pkg: &VerifiedPackage) -> Result<(), WasmError> {
    let dir = root.join(pkg.package_digest.as_str());
    std::fs::create_dir_all(&dir)?;
    let manifest_json = serde_json::to_vec_pretty(&pkg.manifest)?;
    std::fs::write(dir.join("manifest.json"), manifest_json)?;

    let entrypoint_target = dir.join(&pkg.manifest.entrypoint);
    if let Some(parent) = entrypoint_target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(entrypoint_target, &pkg.component_bytes)?;

    if let (Some(wf_rel), Some(wf_bytes)) = (
        pkg.manifest.workflow_path.as_deref(),
        pkg.workflow_bytes.as_deref(),
    ) {
        let wf_target = dir.join(wf_rel);
        if let Some(parent) = wf_target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(wf_target, wf_bytes)?;
    }
    Ok(())
}
