use super::{BoardCommandRecord, BoardIntentOutcome, BoardIntentParams, RunQueries};
use crate::connection::from_sql_i64;
use crate::enum_codec::sha256_hex;
use crate::error::StorageError;
use crate::event_store::EventStore;
use crate::session_store::SessionStore;
use crate::task_store::TaskStore;
use runlark_domain::{
    CommandId, Revision, RunId, SendPromptCommand, SessionCommand, TaskId, Timestamp,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

impl RunQueries {
    /// Updates `pr_url` on both the run and its parent task inside a single transaction.
    ///
    /// # Errors
    /// Returns [`StorageError`] on missing run/task or `SQLite` failure.
    pub fn update_run_and_task_pr_url(
        conn: &mut Connection,
        run_id: &RunId,
        task_id: Option<&TaskId>,
        pr_url: &str,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(mut run) = SessionStore::get_run(&tx, run_id)? else {
            return Err(StorageError::RunNotFound(run_id.clone()));
        };
        run.pr_url = Some(pr_url.to_string());
        run.revision = Revision(run.revision.0 + 1);
        run.updated_at = now;
        SessionStore::save_run(&tx, &run)?;

        if let Some(tid) = task_id.or(run.task_id.as_ref())
            && let Some(mut task) = TaskStore::get_task(&tx, tid)?
        {
            let expected = task.revision;
            task.pr_url = Some(pr_url.to_string());
            task.revision = Revision(expected.0 + 1);
            task.updated_at = now;
            TaskStore::update_task_with_revision(&tx, &task, expected)?;
        }

        let payload = serde_json::json!({
            "run_id": run.id.as_ref(),
            "pr_url": pr_url,
        })
        .to_string();
        EventStore::append_wire_event(&tx, &run.id, "PrCreated", &payload, now)?;
        tx.commit()?;
        Ok(())
    }

    /// Persists an idempotent board intent in `board_commands` before an external effect.
    ///
    /// # Errors
    /// Returns [`StorageError::CommandPayloadConflict`] if the same `command_id` is reused with
    /// a different payload, or [`StorageError::Sqlite`] on failure.
    pub fn persist_board_intent(
        conn: &mut Connection,
        intent: BoardIntentParams<'_>,
        now: Timestamp,
    ) -> Result<BoardIntentOutcome, StorageError> {
        let payload_hash = sha256_hex(intent.payload_json.as_bytes());
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(String, BoardCommandRecord)> = tx
            .query_row(
                concat!(
                    "SELECT payload_hash, command_id, aggregate_type, aggregate_id, command_type, ",
                    "payload, status, error_message, created_at, updated_at ",
                    "FROM board_commands WHERE command_id = ?1"
                ),
                params![intent.command_id.as_ref()],
                |row| {
                    let hash: String = row.get(0)?;
                    let rec = BoardCommandRecord {
                        command_id: CommandId::from(row.get::<_, String>(1)?),
                        aggregate_type: row.get(2)?,
                        aggregate_id: row.get(3)?,
                        command_type: row.get(4)?,
                        payload: row.get(5)?,
                        status: row.get(6)?,
                        error_message: row.get(7)?,
                        created_at: Timestamp(from_sql_i64(row.get::<_, i64>(8)?, 8)?),
                        updated_at: Timestamp(from_sql_i64(row.get::<_, i64>(9)?, 9)?),
                    };
                    Ok((hash, rec))
                },
            )
            .optional()?;

        if let Some((existing_hash, record)) = existing {
            tx.rollback()?;
            if existing_hash != payload_hash {
                return Err(StorageError::CommandPayloadConflict(
                    intent.command_id.clone(),
                ));
            }
            return Ok(BoardIntentOutcome::AlreadyExists { record });
        }

        tx.execute(
            concat!(
                "INSERT INTO board_commands (",
                "command_id, aggregate_type, aggregate_id, command_type, payload, ",
                "payload_hash, expected_revision, status, created_at, updated_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8, ?8)"
            ),
            params![
                intent.command_id.as_ref(),
                intent.aggregate_type,
                intent.aggregate_id,
                intent.command_type,
                intent.payload_json,
                payload_hash,
                Self::to_i64(intent.expected_revision.0)?,
                Self::to_i64(now.0)?,
            ],
        )?;
        tx.commit()?;
        Ok(BoardIntentOutcome::Inserted)
    }

    /// Updates status/error on an existing `board_commands` row.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL error.
    pub fn update_board_intent_status(
        conn: &Connection,
        command_id: &CommandId,
        status: &str,
        error_message: Option<&str>,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        conn.execute(
            "UPDATE board_commands SET status = ?2, error_message = ?3, updated_at = ?4 WHERE command_id = ?1",
            params![
                command_id.as_ref(),
                status,
                error_message,
                Self::to_i64(now.0)?,
            ],
        )?;
        Ok(())
    }

    /// Ensures a queued `SendPrompt` command exists in the outbox for a newly admitted/resumed run.
    ///
    /// # Errors
    /// Enqueues a [`SendPromptCommand`] into the transactional `commands` outbox (`status = 'pending'`).
    ///
    /// # Errors
    /// Returns [`StorageError`] on serialization or SQL failure.
    pub fn enqueue_outbox_prompt(
        conn: &mut Connection,
        command: &SendPromptCommand,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::enqueue_outbox_prompt_in_tx(&tx, command, now)?;
        tx.commit()?;
        Ok(())
    }

    /// Inserts a [`SendPromptCommand`] into `commands` using an already-open transaction/connection.
    ///
    /// # Errors
    /// Returns [`StorageError`] on serialization or SQL failure.
    pub fn enqueue_outbox_prompt_in_tx(
        conn: &Connection,
        command: &SendPromptCommand,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let wrapped = SessionCommand::SendPrompt(command.clone());
        let payload_json = serde_json::to_string(&wrapped)?;
        let canonical_json = serde_json::to_string(&wrapped.canonical_for_hash())?;
        let payload_hash = sha256_hex(canonical_json.as_bytes());
        conn.execute(
            concat!(
                "INSERT OR IGNORE INTO commands (",
                "command_id, session_id, run_id, command_type, payload, ",
                "payload_hash, expected_revision, status, created_at, updated_at) ",
                "VALUES (?1, ?2, ?3, 'send_prompt', ?4, ?5, ?6, 'pending', ?7, ?7)"
            ),
            params![
                command.command_id.as_ref(),
                command.session_id.as_ref(),
                command.run_id.as_ref(),
                payload_json,
                payload_hash,
                Self::to_i64(command.expected_revision.0)?,
                Self::to_i64(now.0)?,
            ],
        )?;
        Ok(())
    }

    /// Clears `active_run_id` on `task_id` when it still points to `run_id`.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn clear_task_active_run_if_matches(
        conn: &Connection,
        task_id: &TaskId,
        run_id: &RunId,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        if let Some(mut task) = TaskStore::get_task(conn, task_id)?
            && task.active_run_id.as_ref() == Some(run_id)
        {
            let expected = task.revision;
            task.active_run_id = None;
            task.revision = Revision(expected.0 + 1);
            task.updated_at = now;
            TaskStore::update_task_with_revision(conn, &task, expected)?;
        }
        Ok(())
    }
}
