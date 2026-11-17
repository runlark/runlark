use crate::connection::{from_sql_i64, to_sql_i64};
use crate::error::StorageError;
use runlark_domain::{EventSeq, RunId, Timestamp};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredEvent {
    pub run_id: RunId,
    pub seq: EventSeq,
    pub event_type: String,
    pub payload: String,
    pub created_at: Timestamp,
}

pub struct EventStore;

impl EventStore {
    /// Appends a new domain event to the append-only event log.
    ///
    /// # Errors
    /// Returns `StorageError` if SQL insertion fails.
    pub fn append_event(
        conn: &Connection,
        run_id: &RunId,
        seq: EventSeq,
        event_type: &str,
        payload: &str,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        conn.execute(
            concat!(
                "INSERT INTO events (run_id, seq, event_type, payload, created_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5)"
            ),
            params![
                run_id.as_ref(),
                to_sql_i64(seq.0),
                event_type,
                payload,
                to_sql_i64(now.0)
            ],
        )?;
        Ok(())
    }

    /// Fetches events for a given run strictly after the specified sequence number.
    /// If `after_seq` is `None`, fetches all events from sequence 0 upward.
    ///
    /// # Errors
    /// Returns `StorageError` if SQL query fails.
    pub fn get_events(
        conn: &Connection,
        run_id: &RunId,
        after_seq: Option<EventSeq>,
    ) -> Result<Vec<StoredEvent>, StorageError> {
        let min_seq = after_seq.map_or(-1, |s| to_sql_i64(s.0));
        let mut stmt = conn.prepare(concat!(
            "SELECT run_id, seq, event_type, payload, created_at ",
            "FROM events ",
            "WHERE run_id = ?1 AND seq > ?2 ",
            "ORDER BY seq ASC"
        ))?;

        let rows = stmt.query_map(params![run_id.as_ref(), min_seq], |row| {
            let r_id: String = row.get(0)?;
            let seq = from_sql_i64(row.get::<_, i64>(1)?);
            let event_type: String = row.get(2)?;
            let payload: String = row.get(3)?;
            let created_at = from_sql_i64(row.get::<_, i64>(4)?);

            Ok(StoredEvent {
                run_id: RunId::from(r_id),
                seq: EventSeq(seq),
                event_type,
                payload,
                created_at: Timestamp(created_at),
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}
