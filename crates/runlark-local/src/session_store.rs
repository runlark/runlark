use crate::connection::{from_sql_i64, to_sql_i64};
use crate::error::StorageError;
use runlark_domain::{
    Revision, Run, RunId, RunStatus, Session, SessionId, SessionStatus, TaskId, Timestamp,
};
use rusqlite::{Connection, OptionalExtension, params};

pub struct SessionStore;

impl SessionStore {
    fn session_status_to_str(status: SessionStatus) -> &'static str {
        match status {
            SessionStatus::Created => "created",
            SessionStatus::Active => "active",
            SessionStatus::Archived => "archived",
        }
    }

    fn str_to_session_status(s: &str) -> SessionStatus {
        match s {
            "active" => SessionStatus::Active,
            "archived" => SessionStatus::Archived,
            _ => SessionStatus::Created,
        }
    }

    fn run_status_to_str(status: RunStatus) -> &'static str {
        match status {
            RunStatus::Queued => "queued",
            RunStatus::Running => "running",
            RunStatus::WaitingForApproval => "waiting_for_approval",
            RunStatus::CancelRequested => "cancel_requested",
            RunStatus::Stopped => "stopped",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
            RunStatus::Interrupted => "interrupted",
        }
    }

    fn str_to_run_status(s: &str) -> RunStatus {
        match s {
            "running" => RunStatus::Running,
            "waiting_for_approval" => RunStatus::WaitingForApproval,
            "cancel_requested" => RunStatus::CancelRequested,
            "stopped" => RunStatus::Stopped,
            "completed" => RunStatus::Completed,
            "failed" => RunStatus::Failed,
            "interrupted" => RunStatus::Interrupted,
            _ => RunStatus::Queued,
        }
    }

    /// Inserts a new session record into storage.
    ///
    /// # Errors
    /// Returns `StorageError` if SQL insertion fails.
    pub fn create_session(conn: &Connection, session: &Session) -> Result<(), StorageError> {
        conn.execute(
            concat!(
                "INSERT INTO sessions (",
                "session_id, task_id, cwd, status, current_revision, ",
                "active_run_id, created_at, updated_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
            ),
            params![
                session.id.as_ref(),
                session.task_id.as_ref(),
                session.cwd,
                Self::session_status_to_str(session.status),
                to_sql_i64(session.current_revision.0)?,
                session.active_run_id.as_ref().map(AsRef::as_ref),
                to_sql_i64(session.created_at.0)?,
                to_sql_i64(session.updated_at.0)?,
            ],
        )?;
        Ok(())
    }

    /// Updates mutable fields of an existing session record.
    ///
    /// # Errors
    /// Returns `StorageError` if SQL update fails.
    pub fn update_session(conn: &Connection, session: &Session) -> Result<(), StorageError> {
        conn.execute(
            concat!(
                "UPDATE sessions SET cwd = ?2, status = ?3, current_revision = ?4, ",
                "active_run_id = ?5, updated_at = ?6 WHERE session_id = ?1"
            ),
            params![
                session.id.as_ref(),
                session.cwd,
                Self::session_status_to_str(session.status),
                to_sql_i64(session.current_revision.0)?,
                session.active_run_id.as_ref().map(AsRef::as_ref),
                to_sql_i64(session.updated_at.0)?,
            ],
        )?;
        Ok(())
    }

    /// Fetches a session by its unique ID.
    ///
    /// # Errors
    /// Returns `StorageError` if SQL query fails.
    pub fn get_session(
        conn: &Connection,
        session_id: &SessionId,
    ) -> Result<Option<Session>, StorageError> {
        conn.query_row(
            concat!(
                "SELECT session_id, task_id, cwd, status, current_revision, active_run_id, created_at, updated_at ",
                "FROM sessions WHERE session_id = ?1"
            ),
            params![session_id.as_ref()],
            |row| {
                let s_id: String = row.get(0)?;
                let t_id: String = row.get(1)?;
                let cwd: String = row.get(2)?;
                let status_str: String = row.get(3)?;
                let rev = from_sql_i64(row.get::<_, i64>(4)?, 4)?;
                let active_r: Option<String> = row.get(5)?;
                let created = from_sql_i64(row.get::<_, i64>(6)?, 6)?;
                let updated = from_sql_i64(row.get::<_, i64>(7)?, 7)?;

                Ok(Session {
                    id: SessionId::from(s_id),
                    task_id: TaskId::from(t_id),
                    cwd,
                    status: Self::str_to_session_status(&status_str),
                    current_revision: Revision(rev),
                    active_run_id: active_r.map(RunId::from),
                    created_at: Timestamp(created),
                    updated_at: Timestamp(updated),
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    /// Lists all sessions ordered by creation time descending.
    ///
    /// # Errors
    /// Returns `StorageError` if SQL query fails.
    pub fn list_sessions(conn: &Connection) -> Result<Vec<Session>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT session_id, task_id, cwd, status, current_revision, active_run_id, created_at, updated_at ",
            "FROM sessions ORDER BY created_at DESC"
        ))?;
        let rows = stmt.query_map([], |row| {
            let s_id: String = row.get(0)?;
            let t_id: String = row.get(1)?;
            let cwd: String = row.get(2)?;
            let status_str: String = row.get(3)?;
            let rev = from_sql_i64(row.get::<_, i64>(4)?, 4)?;
            let active_r: Option<String> = row.get(5)?;
            let created = from_sql_i64(row.get::<_, i64>(6)?, 6)?;
            let updated = from_sql_i64(row.get::<_, i64>(7)?, 7)?;

            Ok(Session {
                id: SessionId::from(s_id),
                task_id: TaskId::from(t_id),
                cwd,
                status: Self::str_to_session_status(&status_str),
                current_revision: Revision(rev),
                active_run_id: active_r.map(RunId::from),
                created_at: Timestamp(created),
                updated_at: Timestamp(updated),
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Upserts a run execution record.
    ///
    /// # Errors
    /// Returns `StorageError` if SQL execution fails.
    pub fn save_run(conn: &Connection, run: &Run) -> Result<(), StorageError> {
        conn.execute(
            concat!(
                "INSERT INTO runs (run_id, session_id, status, worktree_path, revision, created_at, updated_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ",
                "ON CONFLICT(run_id) DO UPDATE SET ",
                "status = excluded.status, ",
                "revision = excluded.revision, ",
                "updated_at = excluded.updated_at"
            ),
            params![
                run.id.as_ref(),
                run.session_id.as_ref(),
                Self::run_status_to_str(run.status),
                run.worktree_path,
                to_sql_i64(run.revision.0)?,
                to_sql_i64(run.created_at.0)?,
                to_sql_i64(run.updated_at.0)?,
            ],
        )?;
        Ok(())
    }

    /// Fetches a run record by its unique ID.
    ///
    /// # Errors
    /// Returns `StorageError` if SQL query fails.
    pub fn get_run(conn: &Connection, run_id: &RunId) -> Result<Option<Run>, StorageError> {
        conn.query_row(
            concat!(
                "SELECT run_id, session_id, status, worktree_path, revision, created_at, updated_at ",
                "FROM runs WHERE run_id = ?1"
            ),
            params![run_id.as_ref()],
            |row| {
                let r_id: String = row.get(0)?;
                let s_id: String = row.get(1)?;
                let status_str: String = row.get(2)?;
                let wt: String = row.get(3)?;
                let rev = from_sql_i64(row.get::<_, i64>(4)?, 4)?;
                let created = from_sql_i64(row.get::<_, i64>(5)?, 5)?;
                let updated = from_sql_i64(row.get::<_, i64>(6)?, 6)?;

                Ok(Run {
                    id: RunId::from(r_id),
                    session_id: SessionId::from(s_id),
                    status: Self::str_to_run_status(&status_str),
                    worktree_path: wt,
                    revision: Revision(rev),
                    created_at: Timestamp(created),
                    updated_at: Timestamp(updated),
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }
}
