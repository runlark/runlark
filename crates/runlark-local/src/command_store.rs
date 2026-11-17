use crate::connection::{from_sql_i64, to_sql_i64};
use crate::error::StorageError;
use runlark_domain::{CommandId, Revision, RunId, SessionCommand, SessionId, Timestamp};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};

/// Result of persisting a command intent.
#[derive(Debug, PartialEq, Eq)]
pub enum PersistOutcome {
    Inserted,
    AlreadyExists { status: String },
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
    /// Computes deterministic SHA-256 hash of a serializable payload.
    fn compute_hash(payload: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(payload.as_bytes());
        hex::encode(hasher.finalize())
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

        let command_type = match command {
            SessionCommand::SendPrompt(_) => "send_prompt",
            SessionCommand::StopRun(_) => "stop_run",
            SessionCommand::ResumeSession(_) => "resume_session",
            SessionCommand::ChangeCwd(_) => "change_cwd",
            SessionCommand::ArchiveSession(_) => "archive_session",
        };

        tx.execute(
            "INSERT INTO commands (
                command_id, session_id, run_id, command_type, payload,
                payload_hash, expected_revision, status, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8, ?8)",
            params![
                command.command_id().as_ref(),
                command.session_id().as_ref(),
                command.run_id().as_ref(),
                command_type,
                payload_json,
                payload_hash,
                to_sql_i64(command.expected_revision().0)?,
                to_sql_i64(now.0)?,
            ],
        )?;

        tx.commit()?;
        Ok(PersistOutcome::Inserted)
    }

    fn ensure_updated(affected: usize, command_id: &CommandId) -> Result<(), StorageError> {
        if affected == 0 {
            return Err(StorageError::InvalidState(format!(
                "command not found: {command_id}"
            )));
        }
        Ok(())
    }

    /// Marks a command as dispatched.
    ///
    /// # Errors
    /// Returns `StorageError::InvalidState` if the command does not exist,
    /// or `StorageError::Sqlite` if SQL update fails.
    pub fn mark_dispatched(
        conn: &Connection,
        command_id: &CommandId,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let affected = conn.execute(
            "UPDATE commands SET status = 'dispatched', dispatch_attempts = dispatch_attempts + 1, updated_at = ?2 WHERE command_id = ?1",
            params![command_id.as_ref(), to_sql_i64(now.0)?],
        )?;
        Self::ensure_updated(affected, command_id)
    }

    /// Marks a command as acknowledged.
    ///
    /// # Errors
    /// Returns `StorageError::InvalidState` if the command does not exist,
    /// or `StorageError::Sqlite` if SQL update fails.
    pub fn mark_acked(
        conn: &Connection,
        command_id: &CommandId,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let affected = conn.execute(
            "UPDATE commands SET status = 'acked', updated_at = ?2 WHERE command_id = ?1",
            params![command_id.as_ref(), to_sql_i64(now.0)?],
        )?;
        Self::ensure_updated(affected, command_id)
    }

    /// Marks a command as failed with an error message.
    ///
    /// # Errors
    /// Returns `StorageError::InvalidState` if the command does not exist,
    /// or `StorageError::Sqlite` if SQL update fails.
    pub fn mark_failed(
        conn: &Connection,
        command_id: &CommandId,
        error: &str,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let affected = conn.execute(
            "UPDATE commands SET status = 'failed', error_message = ?2, updated_at = ?3 WHERE command_id = ?1",
            params![command_id.as_ref(), error, to_sql_i64(now.0)?],
        )?;
        Self::ensure_updated(affected, command_id)
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
            |row| {
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
            },
        )
        .optional()
        .map_err(Into::into)
    }
}
