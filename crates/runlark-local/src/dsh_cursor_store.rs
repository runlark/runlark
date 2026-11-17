use crate::connection::{from_sql_i64, to_sql_i64};
use crate::error::StorageError;
use runlark_domain::{SessionId, Timestamp};
use rusqlite::{Connection, OptionalExtension, params};

/// Tracks the highest DSH `SessionWireEvent.seq` processed per session so reconnects
/// and restarts do not replay stale `turn/start` or `turn/end` frames onto newer runs.
pub struct DshCursorStore;

impl DshCursorStore {
    /// Returns the last processed DSH wire sequence number for `session_id`, if any.
    ///
    /// # Errors
    /// Returns `StorageError` if querying `SQLite` or decoding the integer fails.
    pub fn get_cursor(
        conn: &Connection,
        session_id: &SessionId,
    ) -> Result<Option<u64>, StorageError> {
        let raw: Option<i64> = conn
            .query_row(
                "SELECT last_seq FROM dsh_cursors WHERE session_id = ?1",
                params![session_id.as_ref()],
                |row| row.get(0),
            )
            .optional()?;
        raw.map(|v| from_sql_i64(v, 0).map_err(StorageError::Sqlite))
            .transpose()
    }

    /// Advances the DSH wire cursor for `session_id` to `seq` if `seq` is strictly greater
    /// than the stored cursor (or if no cursor exists yet). Returns `true` when updated.
    ///
    /// # Errors
    /// Returns `StorageError` if `SQLite` insert/update fails.
    pub fn advance_cursor(
        conn: &Connection,
        session_id: &SessionId,
        seq: u64,
        now: Timestamp,
    ) -> Result<bool, StorageError> {
        let sql_seq = to_sql_i64(seq)?;
        let sql_now = to_sql_i64(now.0)?;
        let affected = conn.execute(
            concat!(
                "INSERT INTO dsh_cursors (session_id, last_seq, updated_at) ",
                "VALUES (?1, ?2, ?3) ",
                "ON CONFLICT(session_id) DO UPDATE SET ",
                "last_seq = excluded.last_seq, updated_at = excluded.updated_at ",
                "WHERE excluded.last_seq > dsh_cursors.last_seq"
            ),
            params![session_id.as_ref(), sql_seq, sql_now],
        )?;
        Ok(affected > 0)
    }
}
