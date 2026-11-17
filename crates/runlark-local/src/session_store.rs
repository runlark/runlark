use crate::connection::{from_sql_i64, to_sql_i64};
use crate::enum_codec::{
    str_to_task_kind, str_to_workflow_stage, task_kind_to_str, workflow_stage_to_str,
};
use crate::error::StorageError;
use runlark_domain::{
    ArtifactDigest, BudgetPolicy, CostSummary, PolicyDigest, ProjectId, Revision, Run, RunId,
    RunStatus, Session, SessionId, SessionStatus, TaskId, Timestamp,
};
use rusqlite::{Connection, OptionalExtension, Row, params};

const RUN_SELECT_COLUMNS: &str = concat!(
    "run_id, session_id, status, worktree_path, revision, created_at, updated_at, ",
    "task_id, project_id, workflow_stage, task_kind, workflow_digest, base_sha, head_sha, ",
    "plan_version, policy_digest, budget_json, cost_summary_json, repair_attempts, pr_url"
);

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

    fn row_to_session(row: &Row<'_>) -> rusqlite::Result<Session> {
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
    }

    pub(crate) fn row_to_run(row: &Row<'_>) -> rusqlite::Result<Run> {
        let r_id: String = row.get(0)?;
        let s_id: String = row.get(1)?;
        let status_str: String = row.get(2)?;
        let wt: String = row.get(3)?;
        let rev = from_sql_i64(row.get::<_, i64>(4)?, 4)?;
        let created = from_sql_i64(row.get::<_, i64>(5)?, 5)?;
        let updated = from_sql_i64(row.get::<_, i64>(6)?, 6)?;
        let task_id: Option<String> = row.get(7)?;
        let project_id: Option<String> = row.get(8)?;
        let wf_stage_str: String = row.get(9)?;
        let task_kind_str: String = row.get(10)?;
        let wf_digest_str: Option<String> = row.get(11)?;
        let base_sha: Option<String> = row.get(12)?;
        let head_sha: Option<String> = row.get(13)?;
        let plan_version_raw: i64 = row.get(14)?;
        let policy_digest: Option<String> = row.get(15)?;
        let budget_json: Option<String> = row.get(16)?;
        let cost_summary_json: Option<String> = row.get(17)?;
        let repair_attempts_raw: i64 = row.get(18)?;
        let pr_url: Option<String> = row.get(19)?;

        let workflow_stage = str_to_workflow_stage(&wf_stage_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let task_kind = str_to_task_kind(&task_kind_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(10, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let workflow_digest = wf_digest_str
            .map(ArtifactDigest::parse)
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    11,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
        let plan_version = u32::try_from(plan_version_raw).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                14,
                rusqlite::types::Type::Integer,
                Box::new(e),
            )
        })?;
        let budget_policy = budget_json
            .as_deref()
            .map(serde_json::from_str::<BudgetPolicy>)
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    16,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?
            .unwrap_or_default();
        let cost_summary = cost_summary_json
            .as_deref()
            .map(serde_json::from_str::<CostSummary>)
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    17,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?
            .unwrap_or_default();
        let repair_attempts = u32::try_from(repair_attempts_raw).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                18,
                rusqlite::types::Type::Integer,
                Box::new(e),
            )
        })?;

        Ok(Run {
            id: RunId::from(r_id),
            session_id: SessionId::from(s_id),
            status: Self::str_to_run_status(&status_str),
            worktree_path: wt,
            revision: Revision(rev),
            created_at: Timestamp(created),
            updated_at: Timestamp(updated),
            task_id: task_id.map(TaskId::from),
            project_id: project_id.map(ProjectId::from),
            workflow_stage,
            task_kind,
            workflow_digest,
            base_sha,
            head_sha,
            plan_version,
            policy_digest: policy_digest.map(PolicyDigest::from),
            budget_policy,
            cost_summary,
            repair_attempts,
            pr_url,
        })
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
    /// Returns `StorageError::SessionNotFound` if the session does not exist,
    /// or `StorageError::Sqlite` if SQL update fails.
    pub fn update_session(conn: &Connection, session: &Session) -> Result<(), StorageError> {
        let affected = conn.execute(
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
        if affected == 0 {
            return Err(StorageError::SessionNotFound(session.id.clone()));
        }
        Ok(())
    }

    /// Updates mutable fields of an existing session record using optimistic locking on
    /// `expected_revision`.
    ///
    /// # Errors
    /// Returns `StorageError::InvalidState` if the revision check failed (concurrent update),
    /// or `StorageError::Sqlite` if SQL update fails.
    pub fn update_session_with_revision(
        conn: &Connection,
        session: &Session,
        expected_revision: Revision,
    ) -> Result<(), StorageError> {
        let affected = conn.execute(
            concat!(
                "UPDATE sessions SET cwd = ?2, status = ?3, current_revision = ?4, ",
                "active_run_id = ?5, updated_at = ?6 ",
                "WHERE session_id = ?1 AND current_revision = ?7"
            ),
            params![
                session.id.as_ref(),
                session.cwd,
                Self::session_status_to_str(session.status),
                to_sql_i64(session.current_revision.0)?,
                session.active_run_id.as_ref().map(AsRef::as_ref),
                to_sql_i64(session.updated_at.0)?,
                to_sql_i64(expected_revision.0)?,
            ],
        )?;
        if affected == 0 {
            return Err(StorageError::InvalidState(format!(
                "optimistic concurrency conflict updating session {} at expected revision {}",
                session.id, expected_revision
            )));
        }
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
            Self::row_to_session,
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
        let rows = stmt.query_map([], Self::row_to_session)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Upserts a run execution record (including A1 workflow, SHA, digest, and budget metadata).
    ///
    /// # Errors
    /// Returns `StorageError` if serialization or SQL execution fails.
    pub fn save_run(conn: &Connection, run: &Run) -> Result<(), StorageError> {
        let budget_json = serde_json::to_string(&run.budget_policy)?;
        let cost_summary_json = serde_json::to_string(&run.cost_summary)?;
        conn.execute(
            concat!(
                "INSERT INTO runs (",
                "run_id, session_id, status, worktree_path, revision, created_at, updated_at, ",
                "task_id, project_id, workflow_stage, task_kind, workflow_digest, base_sha, head_sha, ",
                "plan_version, policy_digest, budget_json, cost_summary_json, repair_attempts, pr_url) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20) ",
                "ON CONFLICT(run_id) DO UPDATE SET ",
                "status = excluded.status, ",
                "revision = excluded.revision, ",
                "updated_at = excluded.updated_at, ",
                "task_id = COALESCE(excluded.task_id, runs.task_id), ",
                "project_id = COALESCE(excluded.project_id, runs.project_id), ",
                "workflow_stage = excluded.workflow_stage, ",
                "task_kind = excluded.task_kind, ",
                "workflow_digest = COALESCE(excluded.workflow_digest, runs.workflow_digest), ",
                "base_sha = COALESCE(excluded.base_sha, runs.base_sha), ",
                "head_sha = COALESCE(excluded.head_sha, runs.head_sha), ",
                "plan_version = excluded.plan_version, ",
                "policy_digest = COALESCE(excluded.policy_digest, runs.policy_digest), ",
                "budget_json = excluded.budget_json, ",
                "cost_summary_json = excluded.cost_summary_json, ",
                "repair_attempts = excluded.repair_attempts, ",
                "pr_url = COALESCE(excluded.pr_url, runs.pr_url)"
            ),
            params![
                run.id.as_ref(),
                run.session_id.as_ref(),
                Self::run_status_to_str(run.status),
                run.worktree_path,
                to_sql_i64(run.revision.0)?,
                to_sql_i64(run.created_at.0)?,
                to_sql_i64(run.updated_at.0)?,
                run.task_id.as_ref().map(AsRef::as_ref),
                run.project_id.as_ref().map(AsRef::as_ref),
                workflow_stage_to_str(run.workflow_stage),
                task_kind_to_str(run.task_kind),
                run.workflow_digest.as_ref().map(AsRef::as_ref),
                run.base_sha,
                run.head_sha,
                i64::from(run.plan_version),
                run.policy_digest.as_ref().map(AsRef::as_ref),
                budget_json,
                cost_summary_json,
                i64::from(run.repair_attempts),
                run.pr_url,
            ],
        )?;
        Ok(())
    }

    /// Updates the resolved worktree path on an existing run record.
    ///
    /// # Errors
    /// Returns `StorageError::RunNotFound` if the run does not exist,
    /// or `StorageError::Sqlite` if SQL update fails.
    pub fn update_run_worktree(
        conn: &Connection,
        run_id: &RunId,
        worktree_path: &str,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let affected = conn.execute(
            "UPDATE runs SET worktree_path = ?2, updated_at = ?3 WHERE run_id = ?1",
            params![run_id.as_ref(), worktree_path, to_sql_i64(now.0)?],
        )?;
        if affected == 0 {
            return Err(StorageError::RunNotFound(run_id.clone()));
        }
        Ok(())
    }

    /// Fetches a run record by its unique ID.
    ///
    /// # Errors
    /// Returns `StorageError` if SQL query fails.
    pub fn get_run(conn: &Connection, run_id: &RunId) -> Result<Option<Run>, StorageError> {
        let sql = format!("SELECT {RUN_SELECT_COLUMNS} FROM runs WHERE run_id = ?1");
        conn.query_row(&sql, params![run_id.as_ref()], Self::row_to_run)
            .optional()
            .map_err(Into::into)
    }

    /// Fetches the currently active run for a session, if `active_run_id` is set.
    ///
    /// # Errors
    /// Returns `StorageError` if SQL query fails.
    pub fn get_active_run(
        conn: &Connection,
        session_id: &SessionId,
    ) -> Result<Option<Run>, StorageError> {
        let sql = concat!(
            "SELECT r.run_id, r.session_id, r.status, r.worktree_path, r.revision, r.created_at, r.updated_at, ",
            "r.task_id, r.project_id, r.workflow_stage, r.task_kind, r.workflow_digest, r.base_sha, r.head_sha, ",
            "r.plan_version, r.policy_digest, r.budget_json, r.cost_summary_json, r.repair_attempts, r.pr_url ",
            "FROM runs r INNER JOIN sessions s ON r.run_id = s.active_run_id ",
            "WHERE s.session_id = ?1"
        );
        conn.query_row(sql, params![session_id.as_ref()], Self::row_to_run)
            .optional()
            .map_err(Into::into)
    }

    /// Lists all runs belonging to a session ordered by creation time ascending.
    ///
    /// # Errors
    /// Returns `StorageError` if SQL query fails.
    pub fn list_runs_for_session(
        conn: &Connection,
        session_id: &SessionId,
    ) -> Result<Vec<Run>, StorageError> {
        let sql = format!(
            "SELECT {RUN_SELECT_COLUMNS} FROM runs WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![session_id.as_ref()], Self::row_to_run)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}
