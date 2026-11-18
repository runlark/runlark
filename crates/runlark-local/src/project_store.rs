use crate::connection::{from_sql_i64, to_sql_i64};
use crate::error::StorageError;
use crate::session_store::SessionStore;
use crate::sync_store::SyncStore;
use runlark_domain::{
    BudgetPolicy, PolicyDigest, Project, ProjectId, ProjectOwnershipMode, Revision, Run, Timestamp,
};
use rusqlite::{Connection, OptionalExtension, Row, params};

pub struct ProjectStore;

impl ProjectStore {
    fn row_to_project(row: &Row<'_>) -> rusqlite::Result<Project> {
        let id: String = row.get(0)?;
        let name: String = row.get(1)?;
        let repo_root: String = row.get(2)?;
        let default_branch: String = row.get(3)?;
        let max_concurrent_raw: i64 = row.get(4)?;
        let budget_json: String = row.get(5)?;
        let policy_digest: String = row.get(6)?;
        let rev = from_sql_i64(row.get::<_, i64>(7)?, 7)?;
        let created = from_sql_i64(row.get::<_, i64>(8)?, 8)?;
        let updated = from_sql_i64(row.get::<_, i64>(9)?, 9)?;

        let max_concurrent_runs = u32::try_from(max_concurrent_raw).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Integer,
                Box::new(e),
            )
        })?;
        let budget_policy = serde_json::from_str::<BudgetPolicy>(&budget_json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
        })?;

        Ok(Project {
            id: ProjectId::from(id),
            name,
            repo_root,
            default_branch,
            max_concurrent_runs,
            budget_policy,
            policy_digest: PolicyDigest::from(policy_digest),
            revision: Revision(rev),
            created_at: Timestamp(created),
            updated_at: Timestamp(updated),
        })
    }

    /// Inserts a new [`Project`] into `SQLite`.
    ///
    /// # Errors
    /// Returns [`StorageError`] if serialization or SQL insertion fails.
    pub fn create_project(conn: &Connection, project: &Project) -> Result<(), StorageError> {
        let budget_json = serde_json::to_string(&project.budget_policy)?;
        conn.execute(
            concat!(
                "INSERT INTO projects (",
                "project_id, name, repo_root, default_branch, max_concurrent_runs, ",
                "budget_policy_json, policy_digest, revision, created_at, updated_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
            ),
            params![
                project.id.as_ref(),
                project.name,
                project.repo_root,
                project.default_branch,
                i64::from(project.max_concurrent_runs),
                budget_json,
                project.policy_digest.as_ref(),
                to_sql_i64(project.revision.0)?,
                to_sql_i64(project.created_at.0)?,
                to_sql_i64(project.updated_at.0)?,
            ],
        )?;
        Ok(())
    }

    /// Fetches a [`Project`] by ID.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn get_project(
        conn: &Connection,
        project_id: &ProjectId,
    ) -> Result<Option<Project>, StorageError> {
        conn.query_row(
            concat!(
                "SELECT project_id, name, repo_root, default_branch, max_concurrent_runs, ",
                "budget_policy_json, policy_digest, revision, created_at, updated_at ",
                "FROM projects WHERE project_id = ?1"
            ),
            params![project_id.as_ref()],
            Self::row_to_project,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Lists all projects ordered by creation timestamp ascending.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn list_projects(conn: &Connection) -> Result<Vec<Project>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT project_id, name, repo_root, default_branch, max_concurrent_runs, ",
            "budget_policy_json, policy_digest, revision, created_at, updated_at ",
            "FROM projects ORDER BY created_at ASC, rowid ASC"
        ))?;
        let rows = stmt.query_map([], Self::row_to_project)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Updates a [`Project`] using optimistic concurrency on `expected_revision`.
    ///
    /// # Errors
    /// Returns [`StorageError::InvalidState`] on revision conflict or missing project,
    /// or [`StorageError::Sqlite`] / [`StorageError::Serialization`] on failure.
    pub fn update_project_with_revision(
        conn: &Connection,
        project: &Project,
        expected_revision: Revision,
    ) -> Result<(), StorageError> {
        let budget_json = serde_json::to_string(&project.budget_policy)?;
        let affected = conn.execute(
            concat!(
                "UPDATE projects SET name = ?2, repo_root = ?3, default_branch = ?4, ",
                "max_concurrent_runs = ?5, budget_policy_json = ?6, policy_digest = ?7, ",
                "revision = ?8, updated_at = ?9 ",
                "WHERE project_id = ?1 AND revision = ?10"
            ),
            params![
                project.id.as_ref(),
                project.name,
                project.repo_root,
                project.default_branch,
                i64::from(project.max_concurrent_runs),
                budget_json,
                project.policy_digest.as_ref(),
                to_sql_i64(project.revision.0)?,
                to_sql_i64(project.updated_at.0)?,
                to_sql_i64(expected_revision.0)?,
            ],
        )?;
        if affected == 0 {
            return Err(StorageError::InvalidState(format!(
                "optimistic concurrency conflict updating project {} at expected revision {}",
                project.id, expected_revision
            )));
        }
        Ok(())
    }

    /// Returns the effective [`ProjectOwnershipMode`] (`SoloLocal` or `TeamManaged`) for `project_id`.
    ///
    /// # Errors
    /// Returns [`StorageError`] if querying `project_sync_bindings` fails.
    pub fn get_ownership_mode(
        conn: &Connection,
        project_id: &ProjectId,
    ) -> Result<ProjectOwnershipMode, StorageError> {
        SyncStore::get_ownership_mode(conn, project_id)
    }

    /// Lists all [`Run`] rows associated with `project_id` (directly or via a task in `project_id`).
    ///
    /// # Errors
    /// Returns [`StorageError`] if querying `runs` fails.
    pub fn list_runs_for_project(
        conn: &Connection,
        project_id: &ProjectId,
    ) -> Result<Vec<Run>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT run_id, session_id, status, worktree_path, revision, created_at, updated_at, ",
            "task_id, project_id, workflow_stage, task_kind, workflow_digest, base_sha, head_sha, ",
            "plan_version, policy_digest, budget_json, cost_summary_json, repair_attempts, pr_url ",
            "FROM runs ",
            "WHERE project_id = ?1 OR task_id IN (SELECT task_id FROM tasks WHERE project_id = ?1) ",
            "ORDER BY created_at ASC, rowid ASC"
        ))?;
        let rows = stmt.query_map(params![project_id.as_ref()], SessionStore::row_to_run)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}
