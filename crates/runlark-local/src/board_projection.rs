use crate::attempt_store::AttemptStore;
use crate::cas_store::ArtifactCasStore;
use crate::connection::from_sql_i64;
use crate::decision_store::DecisionStore;
use crate::error::StorageError;
use crate::project_store::ProjectStore;
use crate::session_store::SessionStore;
use crate::task_store::TaskStore;
use runlark_domain::{ArtifactRef, CostSummary, Decision, Project, ProjectId, Run, Task, TaskId};
use rusqlite::{Connection, params};

const RUN_COLUMNS: &str = concat!(
    "run_id, session_id, status, worktree_path, revision, created_at, updated_at, ",
    "task_id, project_id, workflow_stage, task_kind, workflow_digest, base_sha, head_sha, ",
    "plan_version, policy_digest, budget_json, cost_summary_json, repair_attempts, pr_url"
);

/// Read-model projection of a single task with its latest run, cost ledger, decisions, and artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSummary {
    pub task: Task,
    pub latest_run: Option<Run>,
    pub cost_summary: CostSummary,
    pub decisions: Vec<Decision>,
    pub artifacts: Vec<(ArtifactRef, String)>,
}

/// Full Kanban board projection for a project including 2-slot scheduler occupancy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardSnapshot {
    pub project: Project,
    pub tasks: Vec<TaskSummary>,
    pub executing_slots_used: u64,
    pub queued_runs: Vec<Run>,
}

pub struct BoardProjection;

impl BoardProjection {
    /// Counts runs currently occupying an execution slot across the daemon.
    ///
    /// Counts ONLY `running` and `cancel_requested`; `waiting_for_approval` intentionally frees
    /// its execution slot while awaiting human input.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn count_executing_runs(conn: &Connection) -> Result<u64, StorageError> {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM runs WHERE status IN ('running', 'cancel_requested')",
            [],
            |row| row.get(0),
        )?;
        from_sql_i64(count, 0).map_err(Into::into)
    }

    /// Counts runs currently occupying an execution slot for a specific `project_id`.
    ///
    /// Counts ONLY `running` and `cancel_requested`.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn count_executing_runs_for_project(
        conn: &Connection,
        project_id: &ProjectId,
    ) -> Result<u64, StorageError> {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM runs WHERE project_id = ?1 AND status IN ('running', 'cancel_requested')",
            params![project_id.as_ref()],
            |row| row.get(0),
        )?;
        from_sql_i64(count, 0).map_err(Into::into)
    }

    /// Lists all `queued` runs in strict FIFO admission order (`created_at ASC, rowid ASC`).
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn list_queued_runs_fifo(conn: &Connection) -> Result<Vec<Run>, StorageError> {
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE status = 'queued' ORDER BY created_at ASC, rowid ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], SessionStore::row_to_run)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Lists `queued` runs for a specific project in FIFO admission order.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn list_queued_runs_for_project_fifo(
        conn: &Connection,
        project_id: &ProjectId,
    ) -> Result<Vec<Run>, StorageError> {
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE project_id = ?1 AND status = 'queued' ORDER BY created_at ASC, rowid ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![project_id.as_ref()], SessionStore::row_to_run)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Projects a full [`TaskSummary`] for `task_id` from durable `SQLite` state.
    ///
    /// # Errors
    /// Returns [`StorageError::TaskNotFound`] if the task does not exist, or [`StorageError`] on query failure.
    pub fn task_summary(conn: &Connection, task_id: &TaskId) -> Result<TaskSummary, StorageError> {
        let task = TaskStore::get_task(conn, task_id)?
            .ok_or_else(|| StorageError::TaskNotFound(task_id.clone()))?;
        Self::build_task_summary(conn, task)
    }

    fn build_task_summary(conn: &Connection, task: Task) -> Result<TaskSummary, StorageError> {
        let target_run_id = task.active_run_id.as_ref().or(task.latest_run_id.as_ref());
        let latest_run = match target_run_id {
            Some(run_id) => SessionStore::get_run(conn, run_id)?,
            None => None,
        };
        let (cost_summary, artifacts) = match &latest_run {
            Some(run) => {
                let summary = AttemptStore::cost_summary_for_run(
                    conn,
                    &run.id,
                    run.cost_summary.held_micro_usd,
                )?;
                let artifacts = ArtifactCasStore::list_artifacts_for_run(conn, &run.id)?;
                (summary, artifacts)
            }
            None => (CostSummary::default(), Vec::new()),
        };
        let decisions = DecisionStore::list_decisions_for_task(conn, &task.id)?;

        Ok(TaskSummary {
            task,
            latest_run,
            cost_summary,
            decisions,
            artifacts,
        })
    }

    /// Projects the complete [`BoardSnapshot`] for `project_id`.
    ///
    /// # Errors
    /// Returns [`StorageError::ProjectNotFound`] if missing, or [`StorageError`] on query failure.
    pub fn project_board(
        conn: &Connection,
        project_id: &ProjectId,
    ) -> Result<BoardSnapshot, StorageError> {
        let project = ProjectStore::get_project(conn, project_id)?
            .ok_or_else(|| StorageError::ProjectNotFound(project_id.clone()))?;
        let raw_tasks = TaskStore::list_tasks(conn, Some(project_id), None)?;
        let mut tasks = Vec::with_capacity(raw_tasks.len());
        for task in raw_tasks {
            tasks.push(Self::build_task_summary(conn, task)?);
        }
        let executing_slots_used = Self::count_executing_runs_for_project(conn, project_id)?;
        let queued_runs = Self::list_queued_runs_for_project_fifo(conn, project_id)?;

        Ok(BoardSnapshot {
            project,
            tasks,
            executing_slots_used,
            queued_runs,
        })
    }
}
