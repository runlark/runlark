use crate::connection::{from_sql_i64, to_sql_i64, with_immediate_tx};
use crate::draft_store::DraftStore;
use crate::error::StorageError;
use runlark_domain::{
    ArtifactDigest, CommandId, DraftId, ProjectId, Revision, Timestamp, WorkspaceId,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

/// Lifecycle status of a team command queued in `sync_outbox`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncOutboxStatus {
    Pending,
    Dispatched,
    Acked,
    Conflict,
    Failed,
}

impl SyncOutboxStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Dispatched => "dispatched",
            Self::Acked => "acked",
            Self::Conflict => "conflict",
            Self::Failed => "failed",
        }
    }

    fn parse(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "pending" => Ok(Self::Pending),
            "dispatched" => Ok(Self::Dispatched),
            "acked" => Ok(Self::Acked),
            "conflict" => Ok(Self::Conflict),
            "failed" => Ok(Self::Failed),
            other => Err(StorageError::InvalidState(format!(
                "unknown sync_outbox status `{other}`"
            ))),
        }
    }
}

/// Persisted outbound team command awaiting or recording server synchronization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncOutboxEntry {
    pub command_id: CommandId,
    pub workspace_id: WorkspaceId,
    pub project_id: Option<ProjectId>,
    pub draft_id: Option<DraftId>,
    pub command_type: String,
    pub payload_json: String,
    pub expected_revision: Option<Revision>,
    pub status: SyncOutboxStatus,
    pub last_error: Option<String>,
    pub conflict_server_revision: Option<Revision>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Durable `SQLite` store for `sync_outbox` commands.
pub struct SyncOutboxStore;

impl SyncOutboxStore {
    fn row_to_entry(row: &Row<'_>) -> rusqlite::Result<SyncOutboxEntry> {
        let command_id: String = row.get(0)?;
        let workspace_id: String = row.get(1)?;
        let project_id: Option<String> = row.get(2)?;
        let draft_id: Option<String> = row.get(3)?;
        let command_type: String = row.get(4)?;
        let payload_json: String = row.get(5)?;
        let expected_raw: Option<i64> = row.get(6)?;
        let status_str: String = row.get(7)?;
        let last_error: Option<String> = row.get(8)?;
        let conflict_raw: Option<i64> = row.get(9)?;
        let created_at = from_sql_i64(row.get::<_, i64>(10)?, 10)?;
        let updated_at = from_sql_i64(row.get::<_, i64>(11)?, 11)?;

        let expected_revision = expected_raw
            .map(|v| from_sql_i64(v, 6))
            .transpose()?
            .map(Revision);
        let conflict_server_revision = conflict_raw
            .map(|v| from_sql_i64(v, 9))
            .transpose()?
            .map(Revision);
        let status = SyncOutboxStatus::parse(&status_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(e))
        })?;

        Ok(SyncOutboxEntry {
            command_id: CommandId::from(command_id),
            workspace_id: WorkspaceId::from(workspace_id),
            project_id: project_id.map(ProjectId::from),
            draft_id: draft_id.map(DraftId::from),
            command_type,
            payload_json,
            expected_revision,
            status,
            last_error,
            conflict_server_revision,
            created_at: Timestamp(created_at),
            updated_at: Timestamp(updated_at),
        })
    }

    /// Persists a new outbound team command in `sync_outbox` (idempotent on `command_id`).
    ///
    /// # Errors
    /// Returns [`StorageError`] if conversion or SQL execution fails.
    pub fn enqueue(conn: &Connection, entry: &SyncOutboxEntry) -> Result<(), StorageError> {
        let expected_sql = entry
            .expected_revision
            .map(|r| to_sql_i64(r.0))
            .transpose()?;
        let conflict_sql = entry
            .conflict_server_revision
            .map(|r| to_sql_i64(r.0))
            .transpose()?;
        conn.execute(
            concat!(
                "INSERT INTO sync_outbox (",
                "command_id, workspace_id, project_id, draft_id, command_type, payload_json, ",
                "expected_revision, status, last_error, conflict_server_revision, created_at, updated_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) ",
                "ON CONFLICT(command_id) DO NOTHING"
            ),
            params![
                entry.command_id.as_ref(),
                entry.workspace_id.as_ref(),
                entry.project_id.as_ref().map(AsRef::as_ref),
                entry.draft_id.as_ref().map(AsRef::as_ref),
                entry.command_type,
                entry.payload_json,
                expected_sql,
                entry.status.as_str(),
                entry.last_error,
                conflict_sql,
                to_sql_i64(entry.created_at.0)?,
                to_sql_i64(entry.updated_at.0)?,
            ],
        )?;
        Ok(())
    }

    /// Fetches a `sync_outbox` entry by `command_id`.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn get(
        conn: &Connection,
        command_id: &CommandId,
    ) -> Result<Option<SyncOutboxEntry>, StorageError> {
        conn.query_row(
            concat!(
                "SELECT command_id, workspace_id, project_id, draft_id, command_type, payload_json, ",
                "expected_revision, status, last_error, conflict_server_revision, created_at, updated_at ",
                "FROM sync_outbox WHERE command_id = ?1"
            ),
            params![command_id.as_ref()],
            Self::row_to_entry,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Lists `pending` outbox entries for `workspace_id` in FIFO order.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn list_pending_for_workspace(
        conn: &Connection,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<SyncOutboxEntry>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT command_id, workspace_id, project_id, draft_id, command_type, payload_json, ",
            "expected_revision, status, last_error, conflict_server_revision, created_at, updated_at ",
            "FROM sync_outbox WHERE workspace_id = ?1 AND status = 'pending' ",
            "ORDER BY created_at ASC, rowid ASC"
        ))?;
        let rows = stmt.query_map(params![workspace_id.as_ref()], Self::row_to_entry)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Lists all `sync_outbox` entries, optionally filtered by `workspace_id`.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn list_entries(
        conn: &Connection,
        workspace_id: Option<&WorkspaceId>,
    ) -> Result<Vec<SyncOutboxEntry>, StorageError> {
        let cols = concat!(
            "command_id, workspace_id, project_id, draft_id, command_type, payload_json, ",
            "expected_revision, status, last_error, conflict_server_revision, created_at, updated_at"
        );
        if let Some(wid) = workspace_id {
            let sql = format!(
                "SELECT {cols} FROM sync_outbox WHERE workspace_id = ?1 ORDER BY created_at ASC, rowid ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![wid.as_ref()], Self::row_to_entry)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        } else {
            let sql = format!("SELECT {cols} FROM sync_outbox ORDER BY created_at ASC, rowid ASC");
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], Self::row_to_entry)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        }
    }

    fn update_status_with_draft<F, G>(
        conn: &mut Connection,
        command_id: &CommandId,
        update_outbox: F,
        update_draft: G,
    ) -> Result<(), StorageError>
    where
        F: FnOnce(&Connection) -> Result<(), StorageError>,
        G: FnOnce(&Connection, &DraftId) -> Result<(), StorageError>,
    {
        with_immediate_tx(conn, |tx| {
            let existing = Self::get(tx, command_id)?
                .ok_or_else(|| StorageError::SyncOutboxNotFound(command_id.clone()))?;
            update_outbox(tx)?;
            if let Some(ref did) = existing.draft_id
                && DraftStore::get_draft(tx, did)?.is_some()
            {
                update_draft(tx, did)?;
            }
            Ok(())
        })
    }

    /// Marks an outbox command as `acked` and transitions any linked `OfflineDraft` to `Synced`
    /// atomically inside `with_immediate_tx`.
    ///
    /// # Errors
    /// Returns [`StorageError::SyncOutboxNotFound`] if missing, or [`StorageError`] on SQL failure.
    pub fn mark_acked(
        conn: &mut Connection,
        command_id: &CommandId,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let now_sql = to_sql_i64(now.0)?;
        Self::update_status_with_draft(
            conn,
            command_id,
            |tx| {
                tx.execute(
                    concat!(
                        "UPDATE sync_outbox ",
                        "SET status = 'acked', last_error = NULL, conflict_server_revision = NULL, updated_at = ?2 ",
                        "WHERE command_id = ?1"
                    ),
                    params![command_id.as_ref(), now_sql],
                )?;
                Ok(())
            },
            |tx, did| DraftStore::mark_synced(tx, did, now),
        )
    }

    /// Marks an outbox command as `conflict` (`409`) and surfaces the conflict on any linked
    /// `OfflineDraft` atomically inside `with_immediate_tx`.
    ///
    /// # Errors
    /// Returns [`StorageError::SyncOutboxNotFound`] if missing, or [`StorageError`] on SQL failure.
    pub fn mark_conflict(
        conn: &mut Connection,
        command_id: &CommandId,
        server_revision: Option<Revision>,
        reason: &str,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let rev_sql = server_revision.map(|r| to_sql_i64(r.0)).transpose()?;
        let now_sql = to_sql_i64(now.0)?;
        Self::update_status_with_draft(
            conn,
            command_id,
            |tx| {
                tx.execute(
                    concat!(
                        "UPDATE sync_outbox ",
                        "SET status = 'conflict', last_error = ?2, conflict_server_revision = ?3, updated_at = ?4 ",
                        "WHERE command_id = ?1"
                    ),
                    params![command_id.as_ref(), reason, rev_sql, now_sql],
                )?;
                Ok(())
            },
            |tx, did| DraftStore::mark_conflict(tx, did, server_revision, reason, now),
        )
    }

    /// Marks an outbox command as `failed` and updates any linked `OfflineDraft` to `Conflict`
    /// atomically inside `with_immediate_tx`.
    ///
    /// # Errors
    /// Returns [`StorageError::SyncOutboxNotFound`] if missing, or [`StorageError`] on SQL failure.
    pub fn mark_failed(
        conn: &mut Connection,
        command_id: &CommandId,
        reason: &str,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let now_sql = to_sql_i64(now.0)?;
        Self::update_status_with_draft(
            conn,
            command_id,
            |tx| {
                tx.execute(
                    "UPDATE sync_outbox SET status = 'failed', last_error = ?2, updated_at = ?3 WHERE command_id = ?1",
                    params![command_id.as_ref(), reason, now_sql],
                )?;
                Ok(())
            },
            |tx, did| DraftStore::mark_conflict(tx, did, None, reason, now),
        )
    }

    /// Removes pending/conflicted outbox entries linked to `draft_id`.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn delete_for_draft(conn: &Connection, draft_id: &DraftId) -> Result<(), StorageError> {
        conn.execute(
            "DELETE FROM sync_outbox WHERE draft_id = ?1 AND status != 'acked'",
            params![draft_id.as_ref()],
        )?;
        Ok(())
    }
}

/// Tracks CAS artifact digests confirmed uploaded to a remote workspace server.
pub struct SyncedCasStore;

impl SyncedCasStore {
    /// Returns `true` if `digest` is already recorded as uploaded to `workspace_id`.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn is_uploaded(
        conn: &Connection,
        workspace_id: &WorkspaceId,
        digest: &ArtifactDigest,
    ) -> Result<bool, StorageError> {
        let found: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM synced_cas_blobs WHERE workspace_id = ?1 AND digest_sha256 = ?2",
                params![workspace_id.as_ref(), digest.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// Records that `digest` has been uploaded to `workspace_id` CAS.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn mark_uploaded(
        conn: &Connection,
        workspace_id: &WorkspaceId,
        digest: &ArtifactDigest,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        conn.execute(
            concat!(
                "INSERT OR IGNORE INTO synced_cas_blobs (workspace_id, digest_sha256, uploaded_at) ",
                "VALUES (?1, ?2, ?3)"
            ),
            params![workspace_id.as_ref(), digest.as_str(), to_sql_i64(now.0)?],
        )?;
        Ok(())
    }
}
