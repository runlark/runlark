use crate::connection::{from_sql_i64, to_sql_i64};
use crate::error::StorageError;
use runlark_domain::{DomainEvent, EventSeq, RunId, Timestamp};
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
    /// Returns the canonical event type label for a `DomainEvent`.
    #[must_use]
    pub fn domain_event_type(event: &DomainEvent) -> &'static str {
        match event {
            DomainEvent::SessionCreated { .. } => "SessionCreated",
            DomainEvent::RunCreated { .. } => "RunCreated",
            DomainEvent::PromptEnqueued { .. } => "PromptEnqueued",
            DomainEvent::QuestionAsked { .. } => "QuestionAsked",
            DomainEvent::QuestionAnswered { .. } => "QuestionAnswered",
            DomainEvent::RunCancelRequested { .. } => "RunCancelRequested",
            DomainEvent::RunStopped { .. } => "RunStopped",
            DomainEvent::RunCompleted { .. } => "RunCompleted",
            DomainEvent::RunFailed { .. } => "RunFailed",
            DomainEvent::RunInterrupted { .. } => "RunInterrupted",
            DomainEvent::CwdChanged { .. } => "CwdChanged",
            DomainEvent::SessionArchived { .. } => "SessionArchived",
            DomainEvent::SessionResumed { .. } => "SessionResumed",
            DomainEvent::ProjectCreated { .. } => "ProjectCreated",
            DomainEvent::TaskCreated { .. } => "TaskCreated",
            DomainEvent::TaskStageChanged { .. } => "TaskStageChanged",
            DomainEvent::TaskPlanUpdated { .. } => "TaskPlanUpdated",
            DomainEvent::RunStageAdvanced { .. } => "RunStageAdvanced",
            DomainEvent::AttemptRecorded { .. } => "AttemptRecorded",
            DomainEvent::RepairScheduled { .. } => "RepairScheduled",
            DomainEvent::BudgetExceeded { .. } => "BudgetExceeded",
            DomainEvent::UnknownCostRecorded { .. } => "UnknownCostRecorded",
            DomainEvent::DecisionRecorded { .. } => "DecisionRecorded",
            DomainEvent::ArtifactAttached { .. } => "ArtifactAttached",
        }
    }

    /// Allocates the next monotonically increasing sequence number for a run.
    ///
    /// # Errors
    /// Returns `StorageError` if the SQL query fails or sequence overflows `i64`.
    pub fn next_seq(conn: &Connection, run_id: &RunId) -> Result<EventSeq, StorageError> {
        let next: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), -1) + 1 FROM events WHERE run_id = ?1",
            params![run_id.as_ref()],
            |row| row.get(0),
        )?;
        let seq = from_sql_i64(next, 0).map_err(StorageError::Sqlite)?;
        Ok(EventSeq(seq))
    }

    /// Appends a new domain event to the append-only event log with an explicit sequence number.
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
                to_sql_i64(seq.0)?,
                event_type,
                payload,
                to_sql_i64(now.0)?
            ],
        )?;
        Ok(())
    }

    /// Appends a `DomainEvent` to the append-only event log, allocating the next monotonic
    /// sequence number for `run_id`.
    ///
    /// # Errors
    /// Returns `StorageError` if serialization, sequence allocation, or SQL insertion fails.
    pub fn append_domain_event(
        conn: &Connection,
        run_id: &RunId,
        event: &DomainEvent,
        now: Timestamp,
    ) -> Result<StoredEvent, StorageError> {
        let seq = Self::next_seq(conn, run_id)?;
        let event_type = Self::domain_event_type(event).to_string();
        let payload = serde_json::to_string(event)?;
        Self::append_event(conn, run_id, seq, &event_type, &payload, now)?;
        Ok(StoredEvent {
            run_id: run_id.clone(),
            seq,
            event_type,
            payload,
            created_at: now,
        })
    }

    /// Appends a raw external DSH wire event record to the run's event log, allocating
    /// the next monotonic sequence number.
    ///
    /// # Errors
    /// Returns `StorageError` if sequence allocation or SQL insertion fails.
    pub fn append_wire_event(
        conn: &Connection,
        run_id: &RunId,
        event_type: &str,
        payload: &str,
        now: Timestamp,
    ) -> Result<StoredEvent, StorageError> {
        let seq = Self::next_seq(conn, run_id)?;
        Self::append_event(conn, run_id, seq, event_type, payload, now)?;
        Ok(StoredEvent {
            run_id: run_id.clone(),
            seq,
            event_type: event_type.to_string(),
            payload: payload.to_string(),
            created_at: now,
        })
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
        let min_seq = match after_seq {
            Some(s) => to_sql_i64(s.0)?,
            None => -1,
        };
        let mut stmt = conn.prepare(concat!(
            "SELECT run_id, seq, event_type, payload, created_at ",
            "FROM events ",
            "WHERE run_id = ?1 AND seq > ?2 ",
            "ORDER BY seq ASC"
        ))?;

        let rows = stmt.query_map(params![run_id.as_ref(), min_seq], |row| {
            let r_id: String = row.get(0)?;
            let seq = from_sql_i64(row.get::<_, i64>(1)?, 1)?;
            let event_type: String = row.get(2)?;
            let payload: String = row.get(3)?;
            let created_at = from_sql_i64(row.get::<_, i64>(4)?, 4)?;

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
