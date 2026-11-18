use crate::error::ServerError;
use runlark_domain::{ArtifactDigest, ArtifactKind, ArtifactRef, Timestamp};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static CAS_TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Computes the lowercase hex SHA-256 digest of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Content-addressed artifact store for `runlark-server` with strict SHA-256 verification
/// on both upload and download.
#[derive(Debug, Clone)]
pub struct ServerCasStore {
    root: PathBuf,
}

impl ServerCasStore {
    /// Creates a [`ServerCasStore`] rooted at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Returns the filesystem path for `digest`.
    #[must_use]
    pub fn blob_path(&self, digest: &ArtifactDigest) -> PathBuf {
        self.root.join("sha256").join(digest.as_str())
    }

    /// Returns the root path of the CAS store.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Verifies that `bytes` hashes to `expected_digest`, writes atomically via temp file +
    /// `sync_all` + rename, and returns the verified [`ArtifactRef`].
    ///
    /// # Errors
    /// Returns [`ServerError::BadRequest`] if the content digest mismatches `expected_digest`,
    /// or [`ServerError::Io`] if disk persistence fails.
    pub fn put_verified_bytes(
        &self,
        expected_digest: &ArtifactDigest,
        kind: ArtifactKind,
        media_type: &str,
        bytes: &[u8],
        now: Timestamp,
    ) -> Result<ArtifactRef, ServerError> {
        let actual_hex = sha256_hex(bytes);
        if actual_hex != expected_digest.as_str() {
            return Err(ServerError::BadRequest(format!(
                "artifact SHA-256 mismatch: expected {}, actual {actual_hex}",
                expected_digest.as_str()
            )));
        }

        let sha_dir = self.root.join("sha256");
        let tmp_dir = self.root.join("tmp");
        fs::create_dir_all(&sha_dir)?;
        fs::create_dir_all(&tmp_dir)?;

        let final_path = self.blob_path(expected_digest);
        if final_path.exists() {
            let _ = self.get_verified_bytes(expected_digest)?;
        } else {
            let seq = CAS_TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
            let tmp_path = tmp_dir.join(format!("blob-{}-{seq}.tmp", std::process::id()));
            {
                let mut file = File::create(&tmp_path)?;
                file.write_all(bytes)?;
                file.sync_all()?;
            }
            if let Err(err) = fs::rename(&tmp_path, &final_path) {
                let _ = fs::remove_file(&tmp_path);
                if !final_path.exists() {
                    return Err(ServerError::Io(err));
                }
            }
        }

        let size_bytes = u64::try_from(bytes.len())
            .map_err(|_| ServerError::BadRequest("artifact size overflows u64".to_string()))?;

        Ok(ArtifactRef {
            digest_sha256: expected_digest.clone(),
            kind,
            media_type: media_type.to_string(),
            size_bytes,
            created_at: now,
        })
    }

    /// Reads an artifact blob from disk and verifies its SHA-256 digest before returning bytes.
    ///
    /// # Errors
    /// Returns [`ServerError::NotFound`] if missing, [`ServerError::BadRequest`] on hash
    /// corruption, or [`ServerError::Io`] on disk read failure.
    pub fn get_verified_bytes(&self, digest: &ArtifactDigest) -> Result<Vec<u8>, ServerError> {
        let path = self.blob_path(digest);
        if !path.is_file() {
            return Err(ServerError::NotFound(format!(
                "CAS artifact not found: {}",
                digest.as_str()
            )));
        }
        let bytes = fs::read(&path)?;
        let actual = sha256_hex(&bytes);
        if actual != digest.as_str() {
            return Err(ServerError::BadRequest(format!(
                "corrupt CAS artifact {}: expected {}, actual {actual}",
                digest.as_str(),
                digest.as_str()
            )));
        }
        Ok(bytes)
    }
}
