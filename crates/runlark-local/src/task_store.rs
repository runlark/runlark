use crate::connection::{from_sql_i64, to_sql_i64};
use crate::enum_codec::{
    str_to_task_kind, str_to_task_stage, str_to_workflow_stage, task_kind_to_str,
    task_stage_to_str, workflow_stage_to_str,
};
use crate::error::StorageError;
use runlark_domain::{
    BudgetPolicy, ProjectId, Revision, RunId, SessionId, Task, TaskId, TaskStage, Timestamp,
};
use rusqlite::{Connection, OptionalExtension, Row, params};

const TASK_SELECT_COLUMNS: &str = concat!(
    "task_id, project_id, title, description, kind, stage, workflow_stage, plan_text, ",
    "plan_version, base_ref, head_ref, pr_url, active_session_id, active_run_id, ",
    "latest_run_id, budget_override_json, revision, created_at, updated_at"
);

pub struct TaskStore;

impl TaskStore {
    fn row_to_task(row: &Row<'_>) -> rusqlite::Result<Task> {
        let task_id: String = row.get(0)?;
        let project_id: String = row.get(1)?;
        let title: String = row.get(2)?;
        let description: String = row.get(3)?;
        let kind_str: String = row.get(4)?;
        let stage_str: String = row.get(5)?;
        let wf_stage_str: String = row.get(6)?;
        let plan_text: Option<String> = row.get(7)?;
        let plan_ver_raw: i64 = row.get(8)?;
        let base_ref: Option<String> = row.get(9)?;
        let head_ref: Option<String> = row.get(10)?;
        let pr_url: Option<String> = row.get(11)?;
        let active_session_id: Option<String> = row.get(12)?;
        let active_run_id: Option<String> = row.get(13)?;
        let latest_run_id: Option<String> = row.get(14)?;
        let budget_override_json: Option<String> = row.get(15)?;
        let rev = from_sql_i64(row.get::<_, i64>(16)?, 16)?;
        let created = from_sql_i64(row.get::<_, i64>(17)?, 17)?;
        let updated = from_sql_i64(row.get::<_, i64>(18)?, 18)?;

        let kind = str_to_task_kind(&kind_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let stage = str_to_task_stage(&stage_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let workflow_stage = str_to_workflow_stage(&wf_stage_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let plan_version = u32::try_from(plan_ver_raw).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Integer,
                Box::new(e),
            )
        })?;
        let budget_override = budget_override_json
            .as_deref()
            .map(serde_json::from_str::<BudgetPolicy>)
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    15,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;

        Ok(Task {
            id: TaskId::from(task_id),
            project_id: ProjectId::from(project_id),
            title,
            description,
            kind,
            stage,
            workflow_stage,
            plan_text,
            plan_version,
            base_ref,
            head_ref,
            pr_url,
            active_session_id: active_session_id.map(SessionId::from),
            active_run_id: active_run_id.map(RunId::from),
            latest_run_id: latest_run_id.map(RunId::from),
            budget_override,
            revision: Revision(rev),
            created_at: Timestamp(created),
            updated_at: Timestamp(updated),
        })
    }

    /// Inserts a new [`Task`] into `SQLite`.
    ///
    /// # Errors
    /// Returns [`StorageError`] if serialization or SQL insertion fails.
    pub fn create_task(conn: &Connection, task: &Task) -> Result<(), StorageError> {
        let budget_json = task
            .budget_override
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        conn.execute(
            concat!(
                "INSERT INTO tasks (",
                "task_id, project_id, title, description, kind, stage, workflow_stage, ",
                "plan_text, plan_version, base_ref, head_ref, pr_url, active_session_id, ",
                "active_run_id, latest_run_id, budget_override_json, revision, created_at, updated_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)"
            ),
            params![
                task.id.as_ref(),
                task.project_id.as_ref(),
                task.title,
                task.description,
                task_kind_to_str(task.kind),
                task_stage_to_str(task.stage),
                workflow_stage_to_str(task.workflow_stage),
                task.plan_text,
                i64::from(task.plan_version),
                task.base_ref,
                task.head_ref,
                task.pr_url,
                task.active_session_id.as_ref().map(AsRef::as_ref),
                task.active_run_id.as_ref().map(AsRef::as_ref),
                task.latest_run_id.as_ref().map(AsRef::as_ref),
                budget_json,
                to_sql_i64(task.revision.0)?,
                to_sql_i64(task.created_at.0)?,
                to_sql_i64(task.updated_at.0)?,
            ],
        )?;
        Ok(())
    }

    /// Fetches a [`Task`] by ID.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn get_task(conn: &Connection, task_id: &TaskId) -> Result<Option<Task>, StorageError> {
        let sql = format!("SELECT {TASK_SELECT_COLUMNS} FROM tasks WHERE task_id = ?1");
        conn.query_row(&sql, params![task_id.as_ref()], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Lists tasks optionally filtered by `project_id` and/or `stage`.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn list_tasks(
        conn: &Connection,
        project_id: Option<&ProjectId>,
        stage: Option<TaskStage>,
    ) -> Result<Vec<Task>, StorageError> {
        match (project_id, stage) {
            (Some(pid), Some(st)) => {
                let sql = format!(
                    "SELECT {TASK_SELECT_COLUMNS} FROM tasks WHERE project_id = ?1 AND stage = ?2 ORDER BY created_at ASC, rowid ASC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(
                    params![pid.as_ref(), task_stage_to_str(st)],
                    Self::row_to_task,
                )?;
                rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
            }
            (Some(pid), None) => {
                let sql = format!(
                    "SELECT {TASK_SELECT_COLUMNS} FROM tasks WHERE project_id = ?1 ORDER BY created_at ASC, rowid ASC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![pid.as_ref()], Self::row_to_task)?;
                rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
            }
            (None, Some(st)) => {
                let sql = format!(
                    "SELECT {TASK_SELECT_COLUMNS} FROM tasks WHERE stage = ?1 ORDER BY created_at ASC, rowid ASC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![task_stage_to_str(st)], Self::row_to_task)?;
                rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
            }
            (None, None) => {
                let sql = format!(
                    "SELECT {TASK_SELECT_COLUMNS} FROM tasks ORDER BY created_at ASC, rowid ASC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map([], Self::row_to_task)?;
                rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
            }
        }
    }

    /// Updates a [`Task`] with optimistic concurrency check on `expected_revision`.
    ///
    /// # Errors
    /// Returns [`StorageError::InvalidState`] on revision conflict or missing task,
    /// or [`StorageError::Sqlite`] / [`StorageError::Serialization`] on failure.
    pub fn update_task_with_revision(
        conn: &Connection,
        task: &Task,
        expected_revision: Revision,
    ) -> Result<(), StorageError> {
        let budget_json = task
            .budget_override
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let affected = conn.execute(
            concat!(
                "UPDATE tasks SET ",
                "title = ?2, description = ?3, kind = ?4, stage = ?5, workflow_stage = ?6, ",
                "plan_text = ?7, plan_version = ?8, base_ref = ?9, head_ref = ?10, pr_url = ?11, ",
                "active_session_id = ?12, active_run_id = ?13, latest_run_id = ?14, ",
                "budget_override_json = ?15, revision = ?16, updated_at = ?17 ",
                "WHERE task_id = ?1 AND revision = ?18"
            ),
            params![
                task.id.as_ref(),
                task.title,
                task.description,
                task_kind_to_str(task.kind),
                task_stage_to_str(task.stage),
                workflow_stage_to_str(task.workflow_stage),
                task.plan_text,
                i64::from(task.plan_version),
                task.base_ref,
                task.head_ref,
                task.pr_url,
                task.active_session_id.as_ref().map(AsRef::as_ref),
                task.active_run_id.as_ref().map(AsRef::as_ref),
                task.latest_run_id.as_ref().map(AsRef::as_ref),
                budget_json,
                to_sql_i64(task.revision.0)?,
                to_sql_i64(task.updated_at.0)?,
                to_sql_i64(expected_revision.0)?,
            ],
        )?;
        if affected == 0 {
            return Err(StorageError::InvalidState(format!(
                "optimistic concurrency conflict updating task {} at expected revision {}",
                task.id, expected_revision
            )));
        }
        Ok(())
    }
}
