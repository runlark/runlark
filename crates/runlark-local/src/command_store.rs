use crate::connection::{from_sql_i64, to_sql_i64};
use crate::error::StorageError;
use crate::event_store::{EventStore, StoredEvent};
use crate::question_store::QuestionStore;
use crate::session_store::SessionStore;
use runlark_domain::{
    CommandId, DomainEvent, Revision, Run, RunId, Session, SessionCommand, SessionId, Timestamp,
    transition,
};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use sha2::{Digest, Sha256};

/// Result of persisting a raw command intent without applying a transition.
#[derive(Debug, PartialEq, Eq)]
pub enum PersistOutcome {
    Inserted,
    AlreadyExists { status: String },
}

/// Result of atomically persisting a command intent and applying its domain transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandApplyOutcome {
    Applied {
        status: String,
        session: Session,
        run: Option<Run>,
        events: Vec<StoredEvent>,
    },
    AlreadyExists {
        status: String,
        session: Session,
        run: Option<Run>,
    },
}

/// Stored representation of a command in `SQLite`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedCommand {
    pub command_id: CommandId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub command_type: String,
    pub payload: String,
    pub payload_hash: String,
    pub expected_revision: Revision,
    pub status: String,
    pub dispatch_attempts: u32,
    pub error_message: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub struct CommandStore;

impl CommandStore {
    fn compute_hash(payload: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(payload.as_bytes());
        hex::encode(hasher.finalize())
    }

    fn command_type_label(command: &SessionCommand) -> &'static str {
        match command {
            SessionCommand::SendPrompt(_) => "send_prompt",
            SessionCommand::StopRun(_) => "stop_run",
            SessionCommand::ResumeSession(_) => "resume_session",
            SessionCommand::ChangeCwd(_) => "change_cwd",
            SessionCommand::ArchiveSession(_) => "archive_session",
            SessionCommand::AnswerQuestion(_) => "answer_question",
        }
    }

    fn domain_event_run_id(event: &DomainEvent) -> Option<&RunId> {
        match event {
            DomainEvent::RunCreated { run_id, .. }
            | DomainEvent::PromptEnqueued { run_id, .. }
            | DomainEvent::QuestionAsked { run_id, .. }
            | DomainEvent::QuestionAnswered { run_id, .. }
            | DomainEvent::RunCancelRequested { run_id, .. }
            | DomainEvent::RunStopped { run_id, .. }
            | DomainEvent::RunCompleted { run_id, .. }
            | DomainEvent::RunFailed { run_id, .. }
            | DomainEvent::RunInterrupted { run_id, .. }
            | DomainEvent::SessionResumed { run_id, .. } => Some(run_id),
            DomainEvent::SessionCreated { .. }
            | DomainEvent::CwdChanged { .. }
            | DomainEvent::SessionArchived { .. } => None,
        }
    }

    fn insert_command_row(
        conn: &Connection,
        command: &SessionCommand,
        payload_json: &str,
        payload_hash: &str,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        conn.execute(
            "INSERT INTO commands (
                command_id, session_id, run_id, command_type, payload,
                payload_hash, expected_revision, status, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8, ?8)",
            params![
                command.command_id().as_ref(),
                command.session_id().as_ref(),
                command.run_id().as_ref(),
                Self::command_type_label(command),
                payload_json,
                payload_hash,
                to_sql_i64(command.expected_revision().0)?,
                to_sql_i64(now.0)?,
            ],
        )?;
        Ok(())
    }

    fn row_to_persisted_command(row: &Row<'_>) -> rusqlite::Result<PersistedCommand> {
        Ok(PersistedCommand {
            command_id: CommandId::from(row.get::<_, String>(0)?),
            session_id: SessionId::from(row.get::<_, String>(1)?),
            run_id: RunId::from(row.get::<_, String>(2)?),
            command_type: row.get(3)?,
            payload: row.get(4)?,
            payload_hash: row.get(5)?,
            expected_revision: Revision(from_sql_i64(row.get::<_, i64>(6)?, 6)?),
            status: row.get(7)?,
            dispatch_attempts: row.get(8)?,
            error_message: row.get(9)?,
            created_at: Timestamp(from_sql_i64(row.get::<_, i64>(10)?, 10)?),
            updated_at: Timestamp(from_sql_i64(row.get::<_, i64>(11)?, 11)?),
        })
    }

    fn ensure_transitioned(
        conn: &Connection,
        affected: usize,
        command_id: &CommandId,
        target_status: &str,
    ) -> Result<(), StorageError> {
        if affected > 0 {
            return Ok(());
        }

        let existing_status: Option<String> = conn
            .query_row(
                "SELECT status FROM commands WHERE command_id = ?1",
                params![command_id.as_ref()],
                |row| row.get(0),
            )
            .optional()?;

        match existing_status {
            None => Err(StorageError::InvalidState(format!(
                "command not found: {command_id}"
            ))),
            Some(status) => Err(StorageError::InvalidState(format!(
                "invalid status transition from '{status}' to '{target_status}' for command {command_id}"
            ))),
        }
    }

    /// Persists intent into `SQLite` before sending to external effector.
    ///
    /// # Invariants
    /// - Same command ID + same payload returns `AlreadyExists`.
    /// - Same command ID + different payload returns `Err(CommandPayloadConflict)`.
    /// - Fresh command ID inserts with status `pending`.
    ///
    /// # Errors
    /// Returns `StorageError::CommandPayloadConflict` if the command ID exists with different payload,
    /// or `StorageError::Sqlite` on database transaction failure.
    pub fn persist_intent(
        conn: &mut Connection,
        command: &SessionCommand,
        now: Timestamp,
    ) -> Result<PersistOutcome, StorageError> {
        let payload_json = serde_json::to_string(command)?;
        let payload_hash = Self::compute_hash(&payload_json);

        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT payload_hash, status FROM commands WHERE command_id = ?1",
                params![command.command_id().as_ref()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        if let Some((existing_hash, status)) = existing {
            tx.rollback()?;
            if existing_hash == payload_hash {
                return Ok(PersistOutcome::AlreadyExists { status });
            }
            return Err(StorageError::CommandPayloadConflict(
                command.command_id().clone(),
            ));
        }

        Self::insert_command_row(&tx, command, &payload_json, &payload_hash, now)?;
        tx.commit()?;
        Ok(PersistOutcome::Inserted)
    }

    /// Atomically checks command idempotency, loads session and active run, runs
    /// `runlark_domain::transition`, and persists `commands`, `sessions`, `runs`, `questions`,
    /// and `events` inside one `TransactionBehavior::Immediate` transaction.
    ///
    /// # Errors
    /// Returns `StorageError::CommandPayloadConflict`, `StorageError::SessionNotFound`,
    /// `StorageError::Transition`, `StorageError::QuestionNotFound`, or `StorageError::Sqlite`,
    /// rolling back all database changes on any error.
    pub fn persist_command_and_transition(
        conn: &mut Connection,
        command: &SessionCommand,
        now: Timestamp,
    ) -> Result<CommandApplyOutcome, StorageError> {
        let payload_json = serde_json::to_string(command)?;
        let payload_hash = Self::compute_hash(&payload_json);

        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT payload_hash, status FROM commands WHERE command_id = ?1",
                params![command.command_id().as_ref()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        if let Some((existing_hash, status)) = existing {
            if existing_hash != payload_hash {
                tx.rollback()?;
                return Err(StorageError::CommandPayloadConflict(
                    command.command_id().clone(),
                ));
            }
            let session = SessionStore::get_session(&tx, command.session_id())?
                .ok_or_else(|| StorageError::SessionNotFound(command.session_id().clone()))?;
            let run = match &session.active_run_id {
                Some(run_id) => SessionStore::get_run(&tx, run_id)?,
                None => None,
            };
            tx.rollback()?;
            return Ok(CommandApplyOutcome::AlreadyExists {
                status,
                session,
                run,
            });
        }

        let session = SessionStore::get_session(&tx, command.session_id())?
            .ok_or_else(|| StorageError::SessionNotFound(command.session_id().clone()))?;

        let active_run = match &session.active_run_id {
            Some(run_id) => SessionStore::get_run(&tx, run_id)?,
            None if matches!(command, SessionCommand::SendPrompt(_)) => {
                SessionStore::get_run(&tx, command.run_id())?.filter(|r| r.session_id == session.id)
            }
            None => None,
        };

        let outcome = transition(&session, active_run.as_ref(), command, now)?;

        if let SessionCommand::AnswerQuestion(answer_cmd) = command {
            let question = QuestionStore::get_question(&tx, &answer_cmd.question_id)?
                .ok_or_else(|| StorageError::QuestionNotFound(answer_cmd.question_id.clone()))?;
            if question.session_id != session.id || question.run_id != answer_cmd.run_id {
                return Err(StorageError::InvalidState(format!(
                    "question {} does not belong to session {} and run {}",
                    answer_cmd.question_id, session.id, answer_cmd.run_id
                )));
            }
            QuestionStore::mark_answered(&tx, &answer_cmd.question_id, &answer_cmd.answer, now)?;
        }

        Self::insert_command_row(&tx, command, &payload_json, &payload_hash, now)?;
        SessionStore::update_session_with_revision(
            &tx,
            &outcome.new_session,
            session.current_revision,
        )?;
        if let Some(run) = &outcome.new_run {
            SessionStore::save_run(&tx, run)?;
        }

        let mut stored_events = Vec::with_capacity(outcome.events.len());
        for event in &outcome.events {
            let run_id = Self::domain_event_run_id(event).unwrap_or_else(|| command.run_id());
            stored_events.push(EventStore::append_domain_event(&tx, run_id, event, now)?);
        }

        tx.commit()?;
        Ok(CommandApplyOutcome::Applied {
            status: "pending".to_string(),
            session: outcome.new_session,
            run: outcome.new_run,
            events: stored_events,
        })
    }

    /// Marks a command as dispatched (incrementing `dispatch_attempts` and clearing any prior
    /// `error_message` on retry).
    ///
    /// # Errors
    /// Returns `StorageError::InvalidState` if the command does not exist or is already `acked`,
    /// or `StorageError::Sqlite` if SQL update fails.
    pub fn mark_dispatched(
        conn: &Connection,
        command_id: &CommandId,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let affected = conn.execute(
            concat!(
                "UPDATE commands ",
                "SET status = 'dispatched', dispatch_attempts = dispatch_attempts + 1, ",
                "error_message = NULL, updated_at = ?2 ",
                "WHERE command_id = ?1 AND status IN ('pending', 'failed', 'dispatched')"
            ),
            params![command_id.as_ref(), to_sql_i64(now.0)?],
        )?;
        Self::ensure_transitioned(conn, affected, command_id, "dispatched")
    }

    /// Marks a command as acknowledged and clears any stale `error_message`.
    ///
    /// # Errors
    /// Returns `StorageError::InvalidState` if the command does not exist or is already `acked`,
    /// or `StorageError::Sqlite` if SQL update fails.
    pub fn mark_acked(
        conn: &Connection,
        command_id: &CommandId,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let affected = conn.execute(
            concat!(
                "UPDATE commands ",
                "SET status = 'acked', error_message = NULL, updated_at = ?2 ",
                "WHERE command_id = ?1 AND status IN ('pending', 'dispatched', 'failed')"
            ),
            params![command_id.as_ref(), to_sql_i64(now.0)?],
        )?;
        Self::ensure_transitioned(conn, affected, command_id, "acked")
    }

    /// Marks a command as failed with an error message.
    ///
    /// # Errors
    /// Returns `StorageError::InvalidState` if the command does not exist or is already `acked`,
    /// or `StorageError::Sqlite` if SQL update fails.
    pub fn mark_failed(
        conn: &Connection,
        command_id: &CommandId,
        error: &str,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let affected = conn.execute(
            concat!(
                "UPDATE commands ",
                "SET status = 'failed', error_message = ?2, updated_at = ?3 ",
                "WHERE command_id = ?1 AND status IN ('pending', 'dispatched', 'failed')"
            ),
            params![command_id.as_ref(), error, to_sql_i64(now.0)?],
        )?;
        Self::ensure_transitioned(conn, affected, command_id, "failed")
    }

    /// Fetches a persisted command by its ID.
    ///
    /// # Errors
    /// Returns `StorageError` if SQL query fails.
    pub fn get_command(
        conn: &Connection,
        command_id: &CommandId,
    ) -> Result<Option<PersistedCommand>, StorageError> {
        conn.query_row(
            concat!(
                "SELECT command_id, session_id, run_id, command_type, payload, payload_hash, ",
                "expected_revision, status, dispatch_attempts, error_message, created_at, updated_at ",
                "FROM commands WHERE command_id = ?1"
            ),
            params![command_id.as_ref()],
            Self::row_to_persisted_command,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Lists all commands currently in `pending` status in FIFO order.
    ///
    /// # Errors
    /// Returns `StorageError` if SQL query fails.
    pub fn list_pending_commands(conn: &Connection) -> Result<Vec<PersistedCommand>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT command_id, session_id, run_id, command_type, payload, payload_hash, ",
            "expected_revision, status, dispatch_attempts, error_message, created_at, updated_at ",
            "FROM commands WHERE status = 'pending' ORDER BY created_at ASC, rowid ASC"
        ))?;
        let rows = stmt.query_map([], Self::row_to_persisted_command)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Lists all commands currently in `dispatched` status for a run in FIFO order.
    ///
    /// # Errors
    /// Returns `StorageError` if SQL query fails.
    pub fn list_dispatched_for_run(
        conn: &Connection,
        run_id: &RunId,
    ) -> Result<Vec<PersistedCommand>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT command_id, session_id, run_id, command_type, payload, payload_hash, ",
            "expected_revision, status, dispatch_attempts, error_message, created_at, updated_at ",
            "FROM commands WHERE run_id = ?1 AND status = 'dispatched' ",
            "ORDER BY created_at ASC, rowid ASC"
        ))?;
        let rows = stmt.query_map(params![run_id.as_ref()], Self::row_to_persisted_command)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Lists all commands eligible for outbox dispatch or retry (`pending` or `failed`) in FIFO order.
    ///
    /// # Errors
    /// Returns `StorageError` if SQL query fails.
    pub fn list_dispatchable_commands(
        conn: &Connection,
    ) -> Result<Vec<PersistedCommand>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT command_id, session_id, run_id, command_type, payload, payload_hash, ",
            "expected_revision, status, dispatch_attempts, error_message, created_at, updated_at ",
            "FROM commands WHERE status IN ('pending', 'failed') ORDER BY created_at ASC, rowid ASC"
        ))?;
        let rows = stmt.query_map([], Self::row_to_persisted_command)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Counts commands currently in `dispatched` (unreconciled) status for a session.
    ///
    /// # Errors
    /// Returns `StorageError` if SQL query fails.
    pub fn count_dispatched_for_session(
        conn: &Connection,
        session_id: &SessionId,
    ) -> Result<u64, StorageError> {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM commands WHERE session_id = ?1 AND status = 'dispatched'",
            params![session_id.as_ref()],
            |row| row.get(0),
        )?;
        from_sql_i64(count, 0).map_err(Into::into)
    }
}
