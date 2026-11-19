use crate::connection::{from_sql_i64, to_sql_i64};
use crate::error::StorageError;
use runlark_domain::{RunnerId, Timestamp, UserId, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::fmt;

/// Persisted authentication and local runner identity for a collaboration server.
#[derive(Clone, PartialEq, Eq)]
pub struct TeamAuthSession {
    pub server_url: String,
    pub user_id: UserId,
    pub display_name: String,
    pub bearer_token: String,
    pub default_workspace_id: Option<WorkspaceId>,
    pub runner_id: Option<RunnerId>,
    pub runner_name: Option<String>,
    pub updated_at: Timestamp,
}

impl fmt::Debug for TeamAuthSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TeamAuthSession")
            .field("server_url", &self.server_url)
            .field("user_id", &self.user_id)
            .field("display_name", &self.display_name)
            .field("bearer_token", &"[REDACTED]")
            .field("default_workspace_id", &self.default_workspace_id)
            .field("runner_id", &self.runner_id)
            .field("runner_name", &self.runner_name)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

/// Durable `SQLite` store for team authentication sessions and local runner identity.
pub struct TeamAuthStore;

impl TeamAuthStore {
    fn row_to_session(row: &Row<'_>) -> rusqlite::Result<TeamAuthSession> {
        let server_url: String = row.get(0)?;
        let user_id: String = row.get(1)?;
        let display_name: String = row.get(2)?;
        let bearer_token: String = row.get(3)?;
        let default_ws: Option<String> = row.get(4)?;
        let runner_id: Option<String> = row.get(5)?;
        let runner_name: Option<String> = row.get(6)?;
        let updated_at = from_sql_i64(row.get::<_, i64>(7)?, 7)?;
        Ok(TeamAuthSession {
            server_url,
            user_id: UserId::from(user_id),
            display_name,
            bearer_token,
            default_workspace_id: default_ws.map(WorkspaceId::from),
            runner_id: runner_id.map(RunnerId::from),
            runner_name,
            updated_at: Timestamp(updated_at),
        })
    }

    /// Saves or updates a [`TeamAuthSession`].
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn save_session(conn: &Connection, session: &TeamAuthSession) -> Result<(), StorageError> {
        conn.execute(
            concat!(
                "INSERT INTO team_auth_sessions (",
                "server_url, user_id, display_name, bearer_token, default_workspace_id, ",
                "runner_id, runner_name, updated_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ",
                "ON CONFLICT(server_url) DO UPDATE SET ",
                "user_id = excluded.user_id, ",
                "display_name = excluded.display_name, ",
                "bearer_token = excluded.bearer_token, ",
                "default_workspace_id = CASE WHEN team_auth_sessions.user_id = excluded.user_id THEN COALESCE(excluded.default_workspace_id, team_auth_sessions.default_workspace_id) ELSE excluded.default_workspace_id END, ",
                "runner_id = CASE WHEN team_auth_sessions.user_id = excluded.user_id THEN COALESCE(excluded.runner_id, team_auth_sessions.runner_id) ELSE excluded.runner_id END, ",
                "runner_name = CASE WHEN team_auth_sessions.user_id = excluded.user_id THEN COALESCE(excluded.runner_name, team_auth_sessions.runner_name) ELSE excluded.runner_name END, ",
                "updated_at = excluded.updated_at"
            ),
            params![
                session.server_url.trim_end_matches('/'),
                session.user_id.as_ref(),
                session.display_name,
                session.bearer_token,
                session.default_workspace_id.as_ref().map(AsRef::as_ref),
                session.runner_id.as_ref().map(AsRef::as_ref),
                session.runner_name,
                to_sql_i64(session.updated_at.0)?,
            ],
        )?;
        Ok(())
    }

    /// Replaces an auth session row verbatim (including `NULL` fields) for rollback restoration.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn replace_session(
        conn: &Connection,
        session: &TeamAuthSession,
    ) -> Result<(), StorageError> {
        conn.execute(
            concat!(
                "INSERT INTO team_auth_sessions (",
                "server_url, user_id, display_name, bearer_token, default_workspace_id, ",
                "runner_id, runner_name, updated_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ",
                "ON CONFLICT(server_url) DO UPDATE SET ",
                "user_id = excluded.user_id, ",
                "display_name = excluded.display_name, ",
                "bearer_token = excluded.bearer_token, ",
                "default_workspace_id = excluded.default_workspace_id, ",
                "runner_id = excluded.runner_id, ",
                "runner_name = excluded.runner_name, ",
                "updated_at = excluded.updated_at"
            ),
            params![
                session.server_url.trim_end_matches('/'),
                session.user_id.as_ref(),
                session.display_name,
                session.bearer_token,
                session.default_workspace_id.as_ref().map(AsRef::as_ref),
                session.runner_id.as_ref().map(AsRef::as_ref),
                session.runner_name,
                to_sql_i64(session.updated_at.0)?,
            ],
        )?;
        Ok(())
    }

    /// Deletes the auth session for `server_url`, if present.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn delete_by_server(conn: &Connection, server_url: &str) -> Result<(), StorageError> {
        conn.execute(
            "DELETE FROM team_auth_sessions WHERE server_url = ?1",
            params![server_url.trim_end_matches('/')],
        )?;
        Ok(())
    }

    /// Fetches the auth session for a specific `server_url`.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn get_by_server(
        conn: &Connection,
        server_url: &str,
    ) -> Result<Option<TeamAuthSession>, StorageError> {
        conn.query_row(
            concat!(
                "SELECT server_url, user_id, display_name, bearer_token, default_workspace_id, ",
                "runner_id, runner_name, updated_at FROM team_auth_sessions WHERE server_url = ?1"
            ),
            params![server_url.trim_end_matches('/')],
            Self::row_to_session,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Lists all saved team auth sessions ordered by most recently updated.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn list_sessions(conn: &Connection) -> Result<Vec<TeamAuthSession>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT server_url, user_id, display_name, bearer_token, default_workspace_id, ",
            "runner_id, runner_name, updated_at FROM team_auth_sessions ",
            "ORDER BY updated_at DESC, rowid DESC"
        ))?;
        let rows = stmt.query_map([], Self::row_to_session)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Resolves the auth session for `workspace_id` without leaking another server's credentials.
    ///
    /// When `workspace_id` is mapped to a known server in `team_workspaces` or
    /// `project_sync_bindings`, returns only that server's session (or `None` if logged out).
    /// When `workspace_id` is unknown locally, matches `default_workspace_id` or an unambiguous
    /// single saved session.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn resolve_for_workspace(
        conn: &Connection,
        workspace_id: Option<&WorkspaceId>,
    ) -> Result<Option<TeamAuthSession>, StorageError> {
        if let Some(wid) = workspace_id {
            let ws_server: Option<String> = conn
                .query_row(
                    "SELECT server_url FROM team_workspaces WHERE workspace_id = ?1",
                    params![wid.as_ref()],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(url) = ws_server {
                return Self::get_by_server(conn, &url);
            }
            let binding_server: Option<String> = conn
                .query_row(
                    concat!(
                        "SELECT server_url FROM project_sync_bindings ",
                        "WHERE workspace_id = ?1 AND server_url IS NOT NULL LIMIT 1"
                    ),
                    params![wid.as_ref()],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(url) = binding_server {
                return Self::get_by_server(conn, &url);
            }
            let by_default = conn
                .query_row(
                    concat!(
                        "SELECT server_url, user_id, display_name, bearer_token, default_workspace_id, ",
                        "runner_id, runner_name, updated_at FROM team_auth_sessions ",
                        "WHERE default_workspace_id = ?1 ORDER BY updated_at DESC LIMIT 1"
                    ),
                    params![wid.as_ref()],
                    Self::row_to_session,
                )
                .optional()?;
            if by_default.is_some() {
                return Ok(by_default);
            }
            let sessions = Self::list_sessions(conn)?;
            return Ok((sessions.len() == 1)
                .then(|| sessions.into_iter().next())
                .flatten());
        }
        Ok(Self::list_sessions(conn)?.into_iter().next())
    }

    /// Records the registered `runner_id` and `runner_name` on the matching `(server_url, user_id)`
    /// auth session, populating `default_workspace_id` if unset.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn set_runner_identity(
        conn: &Connection,
        server_url: &str,
        user_id: &UserId,
        workspace_id: &WorkspaceId,
        runner_id: &RunnerId,
        runner_name: &str,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        conn.execute(
            concat!(
                "UPDATE team_auth_sessions ",
                "SET runner_id = ?2, runner_name = ?3, ",
                "default_workspace_id = COALESCE(default_workspace_id, ?4), updated_at = ?5 ",
                "WHERE server_url = ?1 AND user_id = ?6"
            ),
            params![
                server_url.trim_end_matches('/'),
                runner_id.as_ref(),
                runner_name,
                workspace_id.as_ref(),
                to_sql_i64(now.0)?,
                user_id.as_ref(),
            ],
        )?;
        Ok(())
    }
}
