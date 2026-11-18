use crate::connection::{from_sql_i64, to_sql_i64};
use crate::error::StorageError;
use runlark_domain::{
    DraftId, DraftKind, DraftSyncState, OfflineDraft, ProjectId, Revision, TaskId, Timestamp,
};
use rusqlite::{Connection, OptionalExtension, Row, params};

/// Durable `SQLite` store for client-side offline drafts and conflict state.
pub struct DraftStore;

impl DraftStore {
    fn kind_to_str(kind: DraftKind) -> &'static str {
        match kind {
            DraftKind::TaskCreate => "task_create",
            DraftKind::TaskComment => "task_comment",
            DraftKind::PlanEdit => "plan_edit",
            DraftKind::DecisionIntent => "decision_intent",
        }
    }

    fn str_to_kind(raw: &str) -> Result<DraftKind, StorageError> {
        match raw {
            "task_create" => Ok(DraftKind::TaskCreate),
            "task_comment" => Ok(DraftKind::TaskComment),
            "plan_edit" => Ok(DraftKind::PlanEdit),
            "decision_intent" => Ok(DraftKind::DecisionIntent),
            other => Err(StorageError::InvalidState(format!(
                "unknown draft_kind `{other}`"
            ))),
        }
    }

    fn state_label(state: &DraftSyncState) -> &'static str {
        match state {
            DraftSyncState::LocalDraft => "local_draft",
            DraftSyncState::PendingSync => "pending_sync",
            DraftSyncState::Conflict { .. } => "conflict",
            DraftSyncState::Synced => "synced",
        }
    }

    fn row_to_draft(row: &Row<'_>) -> rusqlite::Result<OfflineDraft> {
        let draft_id: String = row.get(0)?;
        let project_id: String = row.get(1)?;
        let task_id: Option<String> = row.get(2)?;
        let kind_str: String = row.get(3)?;
        let payload_json: String = row.get(4)?;
        let base_rev_raw: Option<i64> = row.get(5)?;
        let state_str: String = row.get(6)?;
        let conflict_rev_raw: Option<i64> = row.get(7)?;
        let conflict_reason: Option<String> = row.get(8)?;
        let created_at = from_sql_i64(row.get::<_, i64>(9)?, 9)?;
        let updated_at = from_sql_i64(row.get::<_, i64>(10)?, 10)?;

        let kind = Self::str_to_kind(&kind_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let base_revision = base_rev_raw
            .map(|v| from_sql_i64(v, 5))
            .transpose()?
            .map(Revision);
        let server_revision = conflict_rev_raw
            .map(|v| from_sql_i64(v, 7))
            .transpose()?
            .map(Revision);
        let sync_state = match state_str.as_str() {
            "local_draft" => DraftSyncState::LocalDraft,
            "pending_sync" => DraftSyncState::PendingSync,
            "conflict" => DraftSyncState::Conflict {
                server_revision,
                reason: conflict_reason.unwrap_or_else(|| "revision conflict".to_string()),
            },
            "synced" => DraftSyncState::Synced,
            other => {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    6,
                    rusqlite::types::Type::Text,
                    Box::new(StorageError::InvalidState(format!(
                        "unknown draft sync_state `{other}`"
                    ))),
                ));
            }
        };

        Ok(OfflineDraft {
            id: DraftId::from(draft_id),
            project_id: ProjectId::from(project_id),
            task_id: task_id.map(TaskId::from),
            kind,
            payload_json,
            base_revision,
            sync_state,
            created_at: Timestamp(created_at),
            updated_at: Timestamp(updated_at),
        })
    }

    /// Inserts or updates an [`OfflineDraft`] in `SQLite`.
    ///
    /// # Errors
    /// Returns [`StorageError`] if integer conversion or SQL execution fails.
    pub fn save_draft(conn: &Connection, draft: &OfflineDraft) -> Result<(), StorageError> {
        let base_rev = draft.base_revision.map(|r| to_sql_i64(r.0)).transpose()?;
        let (conflict_rev, conflict_reason) = match &draft.sync_state {
            DraftSyncState::Conflict {
                server_revision,
                reason,
            } => (
                server_revision.map(|r| to_sql_i64(r.0)).transpose()?,
                Some(reason.clone()),
            ),
            _ => (None, None),
        };
        conn.execute(
            concat!(
                "INSERT INTO offline_drafts (",
                "draft_id, project_id, task_id, draft_kind, payload_json, base_revision, ",
                "sync_state, conflict_server_revision, conflict_reason, created_at, updated_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) ",
                "ON CONFLICT(draft_id) DO UPDATE SET ",
                "payload_json = excluded.payload_json, ",
                "base_revision = excluded.base_revision, ",
                "sync_state = excluded.sync_state, ",
                "conflict_server_revision = excluded.conflict_server_revision, ",
                "conflict_reason = excluded.conflict_reason, ",
                "updated_at = excluded.updated_at"
            ),
            params![
                draft.id.as_ref(),
                draft.project_id.as_ref(),
                draft.task_id.as_ref().map(AsRef::as_ref),
                Self::kind_to_str(draft.kind),
                draft.payload_json,
                base_rev,
                Self::state_label(&draft.sync_state),
                conflict_rev,
                conflict_reason,
                to_sql_i64(draft.created_at.0)?,
                to_sql_i64(draft.updated_at.0)?,
            ],
        )?;
        Ok(())
    }

    /// Fetches a single [`OfflineDraft`] by ID.
    ///
    /// # Errors
    /// Returns [`StorageError`] if SQL query fails.
    pub fn get_draft(
        conn: &Connection,
        draft_id: &DraftId,
    ) -> Result<Option<OfflineDraft>, StorageError> {
        conn.query_row(
            concat!(
                "SELECT draft_id, project_id, task_id, draft_kind, payload_json, base_revision, ",
                "sync_state, conflict_server_revision, conflict_reason, created_at, updated_at ",
                "FROM offline_drafts WHERE draft_id = ?1"
            ),
            params![draft_id.as_ref()],
            Self::row_to_draft,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Lists all drafts, optionally filtered by `project_id`.
    ///
    /// # Errors
    /// Returns [`StorageError`] if SQL query fails.
    pub fn list_drafts(
        conn: &Connection,
        project_id: Option<&ProjectId>,
    ) -> Result<Vec<OfflineDraft>, StorageError> {
        let base_cols = concat!(
            "draft_id, project_id, task_id, draft_kind, payload_json, base_revision, ",
            "sync_state, conflict_server_revision, conflict_reason, created_at, updated_at"
        );
        if let Some(pid) = project_id {
            let sql = format!(
                "SELECT {base_cols} FROM offline_drafts WHERE project_id = ?1 ORDER BY created_at ASC, rowid ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![pid.as_ref()], Self::row_to_draft)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        } else {
            let sql = format!(
                "SELECT {base_cols} FROM offline_drafts ORDER BY created_at ASC, rowid ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], Self::row_to_draft)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        }
    }

    /// Lists unresolved drafts (`local_draft`, `pending_sync`, `conflict`), optionally filtered by `project_id`.
    ///
    /// # Errors
    /// Returns [`StorageError`] if SQL query fails.
    pub fn list_unresolved_drafts(
        conn: &Connection,
        project_id: Option<&ProjectId>,
    ) -> Result<Vec<OfflineDraft>, StorageError> {
        let all = Self::list_drafts(conn, project_id)?;
        Ok(all
            .into_iter()
            .filter(|d| !matches!(d.sync_state, DraftSyncState::Synced))
            .collect())
    }

    /// Marks an existing draft as `Conflict`, preserving `server_revision` and `reason` visibly.
    ///
    /// # Errors
    /// Returns [`StorageError::DraftNotFound`] if the draft does not exist, or [`StorageError`] on SQL failure.
    pub fn mark_conflict(
        conn: &Connection,
        draft_id: &DraftId,
        server_revision: Option<Revision>,
        reason: &str,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let rev_sql = server_revision.map(|r| to_sql_i64(r.0)).transpose()?;
        let affected = conn.execute(
            concat!(
                "UPDATE offline_drafts ",
                "SET sync_state = 'conflict', conflict_server_revision = ?2, conflict_reason = ?3, updated_at = ?4 ",
                "WHERE draft_id = ?1"
            ),
            params![draft_id.as_ref(), rev_sql, reason, to_sql_i64(now.0)?],
        )?;
        if affected == 0 {
            return Err(StorageError::DraftNotFound(draft_id.clone()));
        }
        Ok(())
    }

    /// Marks an existing draft as `Synced` and clears any prior conflict metadata.
    ///
    /// # Errors
    /// Returns [`StorageError::DraftNotFound`] if the draft does not exist, or [`StorageError`] on SQL failure.
    pub fn mark_synced(
        conn: &Connection,
        draft_id: &DraftId,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let affected = conn.execute(
            concat!(
                "UPDATE offline_drafts ",
                "SET sync_state = 'synced', conflict_server_revision = NULL, conflict_reason = NULL, updated_at = ?2 ",
                "WHERE draft_id = ?1"
            ),
            params![draft_id.as_ref(), to_sql_i64(now.0)?],
        )?;
        if affected == 0 {
            return Err(StorageError::DraftNotFound(draft_id.clone()));
        }
        Ok(())
    }

    /// Marks an existing draft as `PendingSync` (e.g., on manual retry after conflict resolution).
    ///
    /// # Errors
    /// Returns [`StorageError::DraftNotFound`] if the draft does not exist, or [`StorageError`] on SQL failure.
    pub fn mark_pending_sync(
        conn: &Connection,
        draft_id: &DraftId,
        new_base_revision: Option<Revision>,
        new_payload_json: Option<&str>,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let existing = Self::get_draft(conn, draft_id)?
            .ok_or_else(|| StorageError::DraftNotFound(draft_id.clone()))?;
        let updated = OfflineDraft {
            payload_json: new_payload_json
                .map(str::to_string)
                .unwrap_or(existing.payload_json),
            base_revision: new_base_revision.or(existing.base_revision),
            sync_state: DraftSyncState::PendingSync,
            updated_at: now,
            ..existing
        };
        Self::save_draft(conn, &updated)
    }

    /// Deletes an offline draft (e.g., when discarded by the user).
    ///
    /// # Errors
    /// Returns [`StorageError::DraftNotFound`] if missing, or [`StorageError`] on SQL failure.
    pub fn delete_draft(conn: &Connection, draft_id: &DraftId) -> Result<(), StorageError> {
        let affected = conn.execute(
            "DELETE FROM offline_drafts WHERE draft_id = ?1",
            params![draft_id.as_ref()],
        )?;
        if affected == 0 {
            return Err(StorageError::DraftNotFound(draft_id.clone()));
        }
        Ok(())
    }
}
