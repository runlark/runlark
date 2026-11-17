use crate::connection::{from_sql_i64, to_sql_i64};
use crate::error::StorageError;
use runlark_domain::{Question, QuestionId, QuestionStatus, RunId, SessionId, Timestamp};
use rusqlite::{Connection, OptionalExtension, Row, params};

struct RawQuestionRow {
    id: String,
    session_id: String,
    run_id: String,
    prompt: String,
    options_json: String,
    status: String,
    answer_json: Option<String>,
    created_at: u64,
    updated_at: u64,
}

pub struct QuestionStore;

impl QuestionStore {
    fn status_to_str(status: QuestionStatus) -> &'static str {
        match status {
            QuestionStatus::Pending => "pending",
            QuestionStatus::Answered => "answered",
            QuestionStatus::Cancelled => "cancelled",
        }
    }

    fn str_to_status(s: &str) -> Result<QuestionStatus, StorageError> {
        match s {
            "pending" => Ok(QuestionStatus::Pending),
            "answered" => Ok(QuestionStatus::Answered),
            "cancelled" => Ok(QuestionStatus::Cancelled),
            other => Err(StorageError::InvalidState(format!(
                "unknown question status: {other}"
            ))),
        }
    }

    fn read_raw_row(row: &Row<'_>) -> rusqlite::Result<RawQuestionRow> {
        Ok(RawQuestionRow {
            id: row.get(0)?,
            session_id: row.get(1)?,
            run_id: row.get(2)?,
            prompt: row.get(3)?,
            options_json: row.get(4)?,
            status: row.get(5)?,
            answer_json: row.get(6)?,
            created_at: from_sql_i64(row.get::<_, i64>(7)?, 7)?,
            updated_at: from_sql_i64(row.get::<_, i64>(8)?, 8)?,
        })
    }

    fn decode_raw_row(raw: RawQuestionRow) -> Result<Question, StorageError> {
        let options: Vec<String> = serde_json::from_str(&raw.options_json)?;
        let answer: Option<String> = raw
            .answer_json
            .map(|json| serde_json::from_str::<String>(&json))
            .transpose()?;
        let status = Self::str_to_status(&raw.status)?;

        Ok(Question {
            id: QuestionId::from(raw.id),
            session_id: SessionId::from(raw.session_id),
            run_id: RunId::from(raw.run_id),
            prompt: raw.prompt,
            options,
            status,
            answer,
            created_at: Timestamp(raw.created_at),
            updated_at: Timestamp(raw.updated_at),
        })
    }

    /// Persists a new human question or approval request.
    ///
    /// # Errors
    /// Returns `StorageError` if serialization or SQL insertion fails.
    pub fn create_question(conn: &Connection, question: &Question) -> Result<(), StorageError> {
        let options_json = serde_json::to_string(&question.options)?;
        let answer_json = question
            .answer
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        conn.execute(
            concat!(
                "INSERT INTO questions (",
                "question_id, session_id, run_id, prompt, options_json, ",
                "status, answer_json, created_at, updated_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
            ),
            params![
                question.id.as_ref(),
                question.session_id.as_ref(),
                question.run_id.as_ref(),
                question.prompt,
                options_json,
                Self::status_to_str(question.status),
                answer_json,
                to_sql_i64(question.created_at.0)?,
                to_sql_i64(question.updated_at.0)?,
            ],
        )?;
        Ok(())
    }

    /// Fetches a question by its unique ID.
    ///
    /// # Errors
    /// Returns `StorageError` if the SQL query or JSON decoding fails.
    pub fn get_question(
        conn: &Connection,
        question_id: &QuestionId,
    ) -> Result<Option<Question>, StorageError> {
        let raw = conn
            .query_row(
                concat!(
                    "SELECT question_id, session_id, run_id, prompt, options_json, ",
                    "status, answer_json, created_at, updated_at ",
                    "FROM questions WHERE question_id = ?1"
                ),
                params![question_id.as_ref()],
                Self::read_raw_row,
            )
            .optional()?;

        raw.map(Self::decode_raw_row).transpose()
    }

    /// Lists all pending questions ordered by creation time ascending.
    ///
    /// # Errors
    /// Returns `StorageError` if the SQL query or JSON decoding fails.
    pub fn list_pending(conn: &Connection) -> Result<Vec<Question>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT question_id, session_id, run_id, prompt, options_json, ",
            "status, answer_json, created_at, updated_at ",
            "FROM questions WHERE status = 'pending' ORDER BY created_at ASC, rowid ASC"
        ))?;
        let raw_rows = stmt
            .query_map([], Self::read_raw_row)?
            .collect::<Result<Vec<_>, _>>()?;
        raw_rows.into_iter().map(Self::decode_raw_row).collect()
    }

    /// Lists pending questions for a specific session ordered by creation time ascending.
    ///
    /// # Errors
    /// Returns `StorageError` if the SQL query or JSON decoding fails.
    pub fn list_pending_for_session(
        conn: &Connection,
        session_id: &SessionId,
    ) -> Result<Vec<Question>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT question_id, session_id, run_id, prompt, options_json, ",
            "status, answer_json, created_at, updated_at ",
            "FROM questions WHERE session_id = ?1 AND status = 'pending' ",
            "ORDER BY created_at ASC, rowid ASC"
        ))?;
        let raw_rows = stmt
            .query_map(params![session_id.as_ref()], Self::read_raw_row)?
            .collect::<Result<Vec<_>, _>>()?;
        raw_rows.into_iter().map(Self::decode_raw_row).collect()
    }

    /// Lists pending questions for a specific run ordered by creation time ascending.
    ///
    /// # Errors
    /// Returns `StorageError` if the SQL query or JSON decoding fails.
    pub fn list_pending_for_run(
        conn: &Connection,
        run_id: &RunId,
    ) -> Result<Vec<Question>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT question_id, session_id, run_id, prompt, options_json, ",
            "status, answer_json, created_at, updated_at ",
            "FROM questions WHERE run_id = ?1 AND status = 'pending' ",
            "ORDER BY created_at ASC, rowid ASC"
        ))?;
        let raw_rows = stmt
            .query_map(params![run_id.as_ref()], Self::read_raw_row)?
            .collect::<Result<Vec<_>, _>>()?;
        raw_rows.into_iter().map(Self::decode_raw_row).collect()
    }

    /// Marks a pending question as answered and records the JSON-encoded answer.
    ///
    /// # Errors
    /// Returns `StorageError::QuestionNotFound` if the question does not exist,
    /// `StorageError::InvalidState` if the question is no longer pending,
    /// or `StorageError` on serialization or SQL failure.
    pub fn mark_answered(
        conn: &Connection,
        question_id: &QuestionId,
        answer: &str,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let answer_json = serde_json::to_string(answer)?;
        let affected = conn.execute(
            concat!(
                "UPDATE questions SET status = 'answered', answer_json = ?2, updated_at = ?3 ",
                "WHERE question_id = ?1 AND status = 'pending'"
            ),
            params![question_id.as_ref(), answer_json, to_sql_i64(now.0)?],
        )?;
        if affected > 0 {
            return Ok(());
        }

        let existing_status: Option<String> = conn
            .query_row(
                "SELECT status FROM questions WHERE question_id = ?1",
                params![question_id.as_ref()],
                |row| row.get(0),
            )
            .optional()?;
        match existing_status {
            None => Err(StorageError::QuestionNotFound(question_id.clone())),
            Some(status) => Err(StorageError::InvalidState(format!(
                "question {question_id} cannot be answered from status '{status}'"
            ))),
        }
    }

    /// Cancels all remaining pending questions for a run (e.g. when a run terminates).
    ///
    /// # Errors
    /// Returns `StorageError` if SQL execution fails.
    pub fn mark_cancelled_for_run(
        conn: &Connection,
        run_id: &RunId,
        now: Timestamp,
    ) -> Result<usize, StorageError> {
        let affected = conn.execute(
            concat!(
                "UPDATE questions SET status = 'cancelled', updated_at = ?2 ",
                "WHERE run_id = ?1 AND status = 'pending'"
            ),
            params![run_id.as_ref(), to_sql_i64(now.0)?],
        )?;
        Ok(affected)
    }
}
