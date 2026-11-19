use crate::connection::{from_sql_i64, to_sql_i64};
use crate::error::StorageError;
use runlark_domain::{CommentId, RunId, TaskComment, TaskId, Timestamp, UserId};
use rusqlite::{Connection, OptionalExtension, Row, params};

/// Durable `SQLite` store for immutable append-only task comments.
pub struct CommentStore;

impl CommentStore {
    fn row_to_comment(row: &Row<'_>) -> rusqlite::Result<TaskComment> {
        let comment_id: String = row.get(0)?;
        let task_id: String = row.get(1)?;
        let run_id: Option<String> = row.get(2)?;
        let author_user_id: String = row.get(3)?;
        let author_display_name: String = row.get(4)?;
        let body: String = row.get(5)?;
        let created_at = from_sql_i64(row.get::<_, i64>(6)?, 6)?;
        Ok(TaskComment {
            id: CommentId::from(comment_id),
            task_id: TaskId::from(task_id),
            run_id: run_id.map(RunId::from),
            author_user_id: UserId::from(author_user_id),
            author_display_name,
            body,
            created_at: Timestamp(created_at),
        })
    }

    /// Inserts an immutable append-only [`TaskComment`], ignoring identical duplicate `comment_id`
    /// replays and rejecting conflicting content for an existing `comment_id`.
    ///
    /// # Errors
    /// Returns [`StorageError::InvalidState`] if comment body is empty or `comment_id` already
    /// exists with a different task, run, or body, or [`StorageError`] if SQL execution fails.
    pub fn upsert_comment(conn: &Connection, comment: &TaskComment) -> Result<(), StorageError> {
        if comment.body.trim().is_empty() {
            return Err(StorageError::InvalidState(
                "comment body must not be empty".to_string(),
            ));
        }
        let inserted = conn.execute(
            concat!(
                "INSERT OR IGNORE INTO task_comments ",
                "(comment_id, task_id, run_id, author_user_id, author_display_name, body, created_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
            ),
            params![
                comment.id.as_ref(),
                comment.task_id.as_ref(),
                comment.run_id.as_ref().map(AsRef::as_ref),
                comment.author_user_id.as_ref(),
                comment.author_display_name,
                comment.body,
                to_sql_i64(comment.created_at.0)?,
            ],
        )?;
        if inserted == 0
            && let Some(existing) = Self::get_comment(conn, &comment.id)?
            && (existing.task_id != comment.task_id
                || existing.run_id != comment.run_id
                || existing.body != comment.body)
        {
            return Err(StorageError::InvalidState(format!(
                "comment {} already exists with conflicting content",
                comment.id
            )));
        }
        Ok(())
    }

    /// Fetches a single [`TaskComment`] by ID.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn get_comment(
        conn: &Connection,
        comment_id: &CommentId,
    ) -> Result<Option<TaskComment>, StorageError> {
        conn.query_row(
            concat!(
                "SELECT comment_id, task_id, run_id, author_user_id, author_display_name, body, created_at ",
                "FROM task_comments WHERE comment_id = ?1"
            ),
            params![comment_id.as_ref()],
            Self::row_to_comment,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Lists all comments on `task_id` in chronological append order.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn list_comments_for_task(
        conn: &Connection,
        task_id: &TaskId,
    ) -> Result<Vec<TaskComment>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT comment_id, task_id, run_id, author_user_id, author_display_name, body, created_at ",
            "FROM task_comments WHERE task_id = ?1 ORDER BY created_at ASC, comment_id ASC"
        ))?;
        let rows = stmt.query_map(params![task_id.as_ref()], Self::row_to_comment)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}
