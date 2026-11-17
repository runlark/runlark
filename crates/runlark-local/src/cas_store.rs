use crate::connection::{from_sql_i64, to_sql_i64};
use crate::enum_codec::{artifact_kind_to_str, sha256_hex, str_to_artifact_kind};
use crate::error::StorageError;
use runlark_domain::{ArtifactDigest, ArtifactKind, ArtifactRef, RunId, Timestamp};
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Content-addressed artifact store rooted at `<data_dir>/cas`.
///
/// Immutable blobs live at `<cas_root>/sha256/<digest>`, written atomically via temp file +
/// `sync_all` + rename, and verified against their SHA-256 digest on every read.
#[derive(Debug, Clone)]
pub struct ArtifactCasStore {
    cas_root: PathBuf,
}

impl ArtifactCasStore {
    /// Creates an [`ArtifactCasStore`] rooted at `cas_root` (blobs under `cas_root/sha256/<digest>`).
    #[must_use]
    pub fn new(cas_root: impl Into<PathBuf>) -> Self {
        Self {
            cas_root: cas_root.into(),
        }
    }

    /// Creates an [`ArtifactCasStore`] rooted at `<data_dir>/cas`.
    #[must_use]
    pub fn from_data_dir(data_dir: &Path) -> Self {
        Self::new(data_dir.join("cas"))
    }

    /// Returns the root directory of this CAS store.
    #[must_use]
    pub fn cas_root(&self) -> &Path {
        &self.cas_root
    }

    /// Returns the immutable blob path `<cas_root>/sha256/<digest>`.
    #[must_use]
    pub fn blob_path(&self, digest: &ArtifactDigest) -> PathBuf {
        self.cas_root.join("sha256").join(digest.as_str())
    }

    /// Returns `true` if the blob exists on disk at its expected CAS path.
    #[must_use]
    pub fn has_blob(&self, digest: &ArtifactDigest) -> bool {
        self.blob_path(digest).is_file()
    }

    fn row_to_artifact_ref(row: &Row<'_>) -> rusqlite::Result<ArtifactRef> {
        let digest_str: String = row.get(0)?;
        let kind_str: String = row.get(1)?;
        let media_type: String = row.get(2)?;
        let size_bytes = from_sql_i64(row.get::<_, i64>(3)?, 3)?;
        let created = from_sql_i64(row.get::<_, i64>(4)?, 4)?;

        let digest_sha256 = ArtifactDigest::parse(digest_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let kind = str_to_artifact_kind(&kind_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
        })?;

        Ok(ArtifactRef {
            digest_sha256,
            kind,
            media_type,
            size_bytes,
            created_at: Timestamp(created),
        })
    }

    /// Atomically writes `bytes` to `<cas_root>/sha256/<digest>` and records metadata in `SQLite`.
    ///
    /// # Errors
    /// Returns [`StorageError`] if filesystem writes, fsync/rename, digest parsing, or `SQLite`
    /// insertion fails.
    pub fn put_bytes(
        &self,
        conn: &Connection,
        kind: ArtifactKind,
        media_type: &str,
        bytes: &[u8],
        now: Timestamp,
    ) -> Result<ArtifactRef, StorageError> {
        let digest_hex = sha256_hex(bytes);
        let digest = ArtifactDigest::parse(digest_hex)?;

        let sha_dir = self.cas_root.join("sha256");
        let tmp_dir = self.cas_root.join("tmp");
        fs::create_dir_all(&sha_dir)?;
        fs::create_dir_all(&tmp_dir)?;

        let final_path = self.blob_path(&digest);
        if final_path.exists() {
            // Verify existing blob integrity before deduplicating.
            let _ = self.get_bytes(&digest)?;
        } else {
            let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
            let tmp_path = tmp_dir.join(format!("blob-{}-{seq}.tmp", std::process::id()));
            {
                let mut file = File::create(&tmp_path)?;
                file.write_all(bytes)?;
                file.sync_all()?;
            }
            if let Err(err) = fs::rename(&tmp_path, &final_path) {
                let _ = fs::remove_file(&tmp_path);
                if !final_path.exists() {
                    return Err(StorageError::Io(err));
                }
            }
        }

        let size_bytes = u64::try_from(bytes.len()).map_err(|_| {
            StorageError::InvalidState("artifact byte length overflows u64".to_string())
        })?;

        conn.execute(
            concat!(
                "INSERT OR IGNORE INTO artifacts (digest_sha256, kind, media_type, size_bytes, created_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5)"
            ),
            params![
                digest.as_str(),
                artifact_kind_to_str(kind),
                media_type,
                to_sql_i64(size_bytes)?,
                to_sql_i64(now.0)?,
            ],
        )?;

        Ok(ArtifactRef {
            digest_sha256: digest,
            kind,
            media_type: media_type.to_string(),
            size_bytes,
            created_at: now,
        })
    }

    /// Reads an artifact blob from disk and verifies its SHA-256 digest matches `digest`.
    ///
    /// # Errors
    /// Returns [`StorageError::ArtifactNotFound`] if the blob is missing on disk,
    /// [`StorageError::CorruptArtifact`] if the content hash mismatches, or [`StorageError::Io`].
    pub fn get_bytes(&self, digest: &ArtifactDigest) -> Result<Vec<u8>, StorageError> {
        let path = self.blob_path(digest);
        if !path.is_file() {
            return Err(StorageError::ArtifactNotFound(digest.clone()));
        }
        let bytes = fs::read(&path)?;
        let actual = sha256_hex(&bytes);
        if actual != digest.as_str() {
            return Err(StorageError::CorruptArtifact {
                digest: digest.clone(),
                expected: digest.to_string(),
                actual,
            });
        }
        Ok(bytes)
    }

    /// Associates a CAS artifact digest with a run under `label`.
    ///
    /// # Errors
    /// Returns [`StorageError`] if SQL insertion fails.
    pub fn link_to_run(
        conn: &Connection,
        run_id: &RunId,
        digest: &ArtifactDigest,
        label: &str,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        conn.execute(
            concat!(
                "INSERT OR IGNORE INTO run_artifacts (run_id, digest_sha256, label, created_at) ",
                "VALUES (?1, ?2, ?3, ?4)"
            ),
            params![run_id.as_ref(), digest.as_str(), label, to_sql_i64(now.0)?,],
        )?;
        Ok(())
    }

    /// Fetches artifact metadata from `SQLite` by SHA-256 digest.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn get_artifact_meta(
        conn: &Connection,
        digest: &ArtifactDigest,
    ) -> Result<Option<ArtifactRef>, StorageError> {
        conn.query_row(
            concat!(
                "SELECT digest_sha256, kind, media_type, size_bytes, created_at ",
                "FROM artifacts WHERE digest_sha256 = ?1"
            ),
            params![digest.as_str()],
            Self::row_to_artifact_ref,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Lists all artifacts attached to `run_id` along with their labels.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn list_artifacts_for_run(
        conn: &Connection,
        run_id: &RunId,
    ) -> Result<Vec<(ArtifactRef, String)>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT a.digest_sha256, a.kind, a.media_type, a.size_bytes, a.created_at, ra.label ",
            "FROM artifacts a INNER JOIN run_artifacts ra ON a.digest_sha256 = ra.digest_sha256 ",
            "WHERE ra.run_id = ?1 ORDER BY ra.created_at ASC, ra.rowid ASC"
        ))?;
        let rows = stmt.query_map(params![run_id.as_ref()], |row| {
            let meta = Self::row_to_artifact_ref(row)?;
            let label: String = row.get(5)?;
            Ok((meta, label))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}
