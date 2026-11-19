use super::{ProjectSyncBinding, SyncStore, TeamWorkspaceRecord};
use crate::connection::{from_sql_i64, to_sql_i64};
use crate::error::StorageError;
use runlark_domain::{
    CursorSeq, LeaseEpoch, ProjectId, ProjectOwnershipMode, Revision, RunId, RunOwnership,
    RunnerId, RunnerRegistration, RunnerStatus, Timestamp, UserId, Workspace, WorkspaceId,
    WorkspaceMember, WorkspaceRole,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::collections::HashMap;

impl SyncStore {
    fn row_to_binding(row: &Row<'_>) -> rusqlite::Result<ProjectSyncBinding> {
        let project_id: String = row.get(0)?;
        let mode_str: String = row.get(1)?;
        let workspace_id: Option<String> = row.get(2)?;
        let server_url: Option<String> = row.get(3)?;
        let epoch = from_sql_i64(row.get::<_, i64>(4)?, 4)?;
        let cursor = from_sql_i64(row.get::<_, i64>(5)?, 5)?;
        let updated_at = from_sql_i64(row.get::<_, i64>(6)?, 6)?;

        let ownership_mode = match mode_str.as_str() {
            "team_managed" => {
                let wid = workspace_id.ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(StorageError::InvalidState(
                            "team_managed binding missing workspace_id".to_string(),
                        )),
                    )
                })?;
                let url = server_url.ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(StorageError::InvalidState(
                            "team_managed binding missing server_url".to_string(),
                        )),
                    )
                })?;
                ProjectOwnershipMode::TeamManaged {
                    workspace_id: WorkspaceId::from(wid),
                    server_url: url,
                    ownership_epoch: epoch,
                }
            }
            "solo_local" => ProjectOwnershipMode::SoloLocal,
            other => {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Text,
                    Box::new(StorageError::InvalidState(format!(
                        "unknown ownership_mode `{other}`"
                    ))),
                ));
            }
        };

        Ok(ProjectSyncBinding {
            project_id: ProjectId::from(project_id),
            ownership_mode,
            last_cursor_seq: CursorSeq(cursor),
            updated_at: Timestamp(updated_at),
        })
    }

    fn row_to_workspace_record(row: &Row<'_>) -> rusqlite::Result<TeamWorkspaceRecord> {
        let workspace_id: String = row.get(0)?;
        let server_url: String = row.get(1)?;
        let name: String = row.get(2)?;
        let slug: String = row.get(3)?;
        let created_by: String = row.get(4)?;
        let revision = from_sql_i64(row.get::<_, i64>(5)?, 5)?;
        let cursor = from_sql_i64(row.get::<_, i64>(6)?, 6)?;
        let created_at = from_sql_i64(row.get::<_, i64>(7)?, 7)?;
        let updated_at = from_sql_i64(row.get::<_, i64>(8)?, 8)?;
        Ok(TeamWorkspaceRecord {
            workspace: Workspace {
                id: WorkspaceId::from(workspace_id),
                name,
                slug,
                created_by: UserId::from(created_by),
                revision: Revision(revision),
                created_at: Timestamp(created_at),
                updated_at: Timestamp(updated_at),
            },
            server_url,
            last_cursor_seq: CursorSeq(cursor),
        })
    }

    /// Fetches the sync binding for `project_id`, if any.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn get_project_binding(
        conn: &Connection,
        project_id: &ProjectId,
    ) -> Result<Option<ProjectSyncBinding>, StorageError> {
        conn.query_row(
            concat!(
                "SELECT project_id, ownership_mode, workspace_id, server_url, ownership_epoch, ",
                "last_cursor_seq, updated_at FROM project_sync_bindings WHERE project_id = ?1"
            ),
            params![project_id.as_ref()],
            Self::row_to_binding,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Lists all project sync bindings.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn list_project_bindings(
        conn: &Connection,
    ) -> Result<Vec<ProjectSyncBinding>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT project_id, ownership_mode, workspace_id, server_url, ownership_epoch, ",
            "last_cursor_seq, updated_at FROM project_sync_bindings ORDER BY project_id ASC"
        ))?;
        let rows = stmt.query_map([], Self::row_to_binding)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Returns the effective [`ProjectOwnershipMode`] for `project_id` (`SoloLocal` by default).
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn get_ownership_mode(
        conn: &Connection,
        project_id: &ProjectId,
    ) -> Result<ProjectOwnershipMode, StorageError> {
        Ok(Self::get_project_binding(conn, project_id)?
            .map_or(ProjectOwnershipMode::SoloLocal, |b| b.ownership_mode))
    }

    /// Lists all cached team workspaces.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn list_workspaces(conn: &Connection) -> Result<Vec<TeamWorkspaceRecord>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT workspace_id, server_url, name, slug, created_by, revision, last_cursor_seq, ",
            "created_at, updated_at FROM team_workspaces ORDER BY created_at ASC, workspace_id ASC"
        ))?;
        let rows = stmt.query_map([], Self::row_to_workspace_record)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Returns the latest applied `CursorSeq` for `workspace_id` (`0` if never synced).
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn get_workspace_cursor(
        conn: &Connection,
        workspace_id: &WorkspaceId,
    ) -> Result<CursorSeq, StorageError> {
        let ws_cursor: Option<i64> = conn
            .query_row(
                "SELECT last_cursor_seq FROM team_workspaces WHERE workspace_id = ?1",
                params![workspace_id.as_ref()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(raw) = ws_cursor {
            return Ok(CursorSeq(from_sql_i64(raw, 0)?));
        }
        let max_binding: Option<i64> = conn.query_row(
            "SELECT MAX(last_cursor_seq) FROM project_sync_bindings WHERE workspace_id = ?1",
            params![workspace_id.as_ref()],
            |row| row.get(0),
        )?;
        Ok(CursorSeq(match max_binding {
            Some(v) => from_sql_i64(v, 0)?,
            None => 0,
        }))
    }

    /// Lists cached members of `workspace_id`.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn list_members_for_workspace(
        conn: &Connection,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<WorkspaceMember>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT workspace_id, user_id, display_name, role, joined_at ",
            "FROM team_members WHERE workspace_id = ?1 ORDER BY joined_at ASC, user_id ASC"
        ))?;
        let rows = stmt.query_map(params![workspace_id.as_ref()], |row| {
            let wid: String = row.get(0)?;
            let uid: String = row.get(1)?;
            let display_name: String = row.get(2)?;
            let role_str: String = row.get(3)?;
            let joined_at = from_sql_i64(row.get::<_, i64>(4)?, 4)?;
            let role = WorkspaceRole::from_str_name(&role_str).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    Box::new(StorageError::InvalidState(format!(
                        "unknown workspace role `{role_str}`"
                    ))),
                )
            })?;
            Ok(WorkspaceMember {
                workspace_id: WorkspaceId::from(wid),
                user_id: UserId::from(uid),
                display_name,
                role,
                joined_at: Timestamp(joined_at),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Lists cached runners registered in `workspace_id`.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn list_runners_for_workspace(
        conn: &Connection,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<RunnerRegistration>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT runner_id, workspace_id, owner_user_id, name, status, last_heartbeat_at, created_at ",
            "FROM team_runners WHERE workspace_id = ?1 ORDER BY created_at ASC, runner_id ASC"
        ))?;
        let rows = stmt.query_map(params![workspace_id.as_ref()], |row| {
            let rid: String = row.get(0)?;
            let wid: String = row.get(1)?;
            let owner: String = row.get(2)?;
            let name: String = row.get(3)?;
            let status_str: String = row.get(4)?;
            let last_hb = from_sql_i64(row.get::<_, i64>(5)?, 5)?;
            let created_at = from_sql_i64(row.get::<_, i64>(6)?, 6)?;
            let status = RunnerStatus::from_str_name(&status_str).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    Box::new(StorageError::InvalidState(format!(
                        "unknown runner status `{status_str}`"
                    ))),
                )
            })?;
            Ok(RunnerRegistration {
                id: RunnerId::from(rid),
                workspace_id: WorkspaceId::from(wid),
                owner_user_id: UserId::from(owner),
                name,
                status,
                last_heartbeat_at: Timestamp(last_hb),
                created_at: Timestamp(created_at),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Fetches the exclusive [`RunOwnership`] lease for `run_id`, if recorded.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn get_run_ownership(
        conn: &Connection,
        run_id: &RunId,
    ) -> Result<Option<RunOwnership>, StorageError> {
        conn.query_row(
            concat!(
                "SELECT assigned_runner_id, lease_epoch, runner_unreachable ",
                "FROM run_ownership WHERE run_id = ?1"
            ),
            params![run_id.as_ref()],
            |row| {
                let runner: Option<String> = row.get(0)?;
                let epoch = from_sql_i64(row.get::<_, i64>(1)?, 1)?;
                let unreachable_int: i64 = row.get(2)?;
                Ok(RunOwnership {
                    assigned_runner_id: runner.map(RunnerId::from),
                    lease_epoch: LeaseEpoch(epoch),
                    runner_unreachable: unreachable_int != 0,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    /// Lists run ownership leases for all runs belonging to `project_id`.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn list_run_ownerships_for_project(
        conn: &Connection,
        project_id: &ProjectId,
    ) -> Result<HashMap<RunId, RunOwnership>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT ro.run_id, ro.assigned_runner_id, ro.lease_epoch, ro.runner_unreachable ",
            "FROM run_ownership ro ",
            "INNER JOIN runs r ON r.run_id = ro.run_id ",
            "WHERE r.project_id = ?1"
        ))?;
        let rows = stmt.query_map(params![project_id.as_ref()], |row| {
            let run_id: String = row.get(0)?;
            let runner: Option<String> = row.get(1)?;
            let epoch = from_sql_i64(row.get::<_, i64>(2)?, 2)?;
            let unreachable_int: i64 = row.get(3)?;
            Ok((
                RunId::from(run_id),
                RunOwnership {
                    assigned_runner_id: runner.map(RunnerId::from),
                    lease_epoch: LeaseEpoch(epoch),
                    runner_unreachable: unreachable_int != 0,
                },
            ))
        })?;
        rows.collect::<Result<HashMap<_, _>, _>>()
            .map_err(Into::into)
    }

    /// Upserts workspace metadata and cursor sequence into `team_workspaces`.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn upsert_workspace_metadata(
        conn: &Connection,
        server_url: &str,
        workspace: &Workspace,
        cursor_seq: CursorSeq,
    ) -> Result<(), StorageError> {
        conn.execute(
            concat!(
                "INSERT INTO team_workspaces (",
                "workspace_id, server_url, name, slug, created_by, revision, last_cursor_seq, created_at, updated_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ",
                "ON CONFLICT(workspace_id) DO UPDATE SET ",
                "server_url = excluded.server_url, ",
                "name = excluded.name, ",
                "slug = excluded.slug, ",
                "revision = excluded.revision, ",
                "last_cursor_seq = excluded.last_cursor_seq, ",
                "updated_at = excluded.updated_at"
            ),
            params![
                workspace.id.as_ref(),
                server_url.trim_end_matches('/'),
                workspace.name,
                workspace.slug,
                workspace.created_by.as_ref(),
                to_sql_i64(workspace.revision.0)?,
                to_sql_i64(cursor_seq.0)?,
                to_sql_i64(workspace.created_at.0)?,
                to_sql_i64(workspace.updated_at.0)?,
            ],
        )?;
        Ok(())
    }

    /// Upserts a workspace member record.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn upsert_member(conn: &Connection, member: &WorkspaceMember) -> Result<(), StorageError> {
        conn.execute(
            concat!(
                "INSERT INTO team_members (workspace_id, user_id, display_name, role, joined_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5) ",
                "ON CONFLICT(workspace_id, user_id) DO UPDATE SET ",
                "display_name = excluded.display_name, ",
                "role = excluded.role"
            ),
            params![
                member.workspace_id.as_ref(),
                member.user_id.as_ref(),
                member.display_name,
                member.role.as_str(),
                to_sql_i64(member.joined_at.0)?,
            ],
        )?;
        Ok(())
    }

    /// Upserts a registered team runner record.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn upsert_runner(
        conn: &Connection,
        runner: &RunnerRegistration,
    ) -> Result<(), StorageError> {
        conn.execute(
            concat!(
                "INSERT INTO team_runners (",
                "runner_id, workspace_id, owner_user_id, name, status, last_heartbeat_at, created_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ",
                "ON CONFLICT(runner_id) DO UPDATE SET ",
                "name = excluded.name, ",
                "status = excluded.status, ",
                "last_heartbeat_at = excluded.last_heartbeat_at"
            ),
            params![
                runner.id.as_ref(),
                runner.workspace_id.as_ref(),
                runner.owner_user_id.as_ref(),
                runner.name,
                runner.status.as_str(),
                to_sql_i64(runner.last_heartbeat_at.0)?,
                to_sql_i64(runner.created_at.0)?,
            ],
        )?;
        Ok(())
    }
}
