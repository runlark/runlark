mod enqueue;
mod intent;

use crate::connection::to_sql_i64;
use crate::error::StorageError;
use crate::project_store::ProjectStore;
use crate::session_store::SessionStore;
use runlark_domain::{
    ArtifactDigest, CommandId, ProjectId, Revision, Run, Task, TaskId, Timestamp,
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

const RUN_COLUMNS: &str = concat!(
    "run_id, session_id, status, worktree_path, revision, created_at, updated_at, ",
    "task_id, project_id, workflow_stage, task_kind, workflow_digest, base_sha, head_sha, ",
    "plan_version, policy_digest, budget_json, cost_summary_json, repair_attempts, pr_url"
);

/// Input parameters for atomically starting a task run in `Queued` state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnqueueTaskRunParams {
    pub command_id: CommandId,
    pub task_id: TaskId,
    pub run_id: RunIdRef,
    pub session_id: runlark_domain::SessionId,
    pub prompt: String,
    pub workflow_digest: Option<ArtifactDigest>,
    pub base_sha: Option<String>,
    pub expected_revision: Option<Revision>,
}

type RunIdRef = runlark_domain::RunId;

/// Outcome of atomically enqueuing a task run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnqueueTaskRunOutcome {
    Enqueued {
        task: Task,
        session: runlark_domain::Session,
        run: Run,
        events: Vec<crate::event_store::StoredEvent>,
    },
    AlreadyExists {
        status: String,
        task: Task,
        run: Option<Run>,
    },
}

/// Stored row from `board_commands`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardCommandRecord {
    pub command_id: CommandId,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub command_type: String,
    pub payload: String,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Metadata describing a board-level external effect intent.
#[derive(Debug, Clone, Copy)]
pub struct BoardIntentParams<'a> {
    pub command_id: &'a CommandId,
    pub aggregate_type: &'a str,
    pub aggregate_id: &'a str,
    pub command_type: &'a str,
    pub payload_json: &'a str,
    pub expected_revision: Revision,
}

/// Result of persisting a board-level external effect intent (e.g., PR creation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardIntentOutcome {
    Inserted,
    AlreadyExists { record: BoardCommandRecord },
}

pub struct RunQueries;

impl RunQueries {
    /// Lists all runs ordered by creation timestamp descending.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn list_runs(conn: &Connection) -> Result<Vec<Run>, StorageError> {
        let sql = format!("SELECT {RUN_COLUMNS} FROM runs ORDER BY created_at DESC, rowid DESC");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], SessionStore::row_to_run)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Lists all runs for `task_id` ordered by creation timestamp ascending.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn list_runs_for_task(
        conn: &Connection,
        task_id: &TaskId,
    ) -> Result<Vec<Run>, StorageError> {
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE task_id = ?1 ORDER BY created_at ASC, rowid ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![task_id.as_ref()], SessionStore::row_to_run)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Lists all runs currently consuming an execution slot (`running` or `cancel_requested`).
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn list_executing_runs(conn: &Connection) -> Result<Vec<Run>, StorageError> {
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE status IN ('running', 'cancel_requested') ORDER BY created_at ASC, rowid ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], SessionStore::row_to_run)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Resolves a project ID either from explicit hint or the first created project.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn resolve_default_project_id(
        conn: &Connection,
        explicit: Option<&ProjectId>,
    ) -> Result<Option<ProjectId>, StorageError> {
        if let Some(id) = explicit {
            return Ok(Some(id.clone()));
        }
        let projects = ProjectStore::list_projects(conn)?;
        Ok(projects.into_iter().next().map(|p| p.id))
    }

    pub(super) fn to_i64(val: u64) -> Result<i64, StorageError> {
        to_sql_i64(val)
    }
}
