use crate::connection::{from_sql_i64, to_sql_i64};
use crate::enum_codec::{str_to_workflow_stage, workflow_stage_to_str};
use crate::error::StorageError;
use runlark_domain::{
    EventSeq, LeaseEpoch, RunId, RunStatus, RunnerId, RunnerRunReport, Timestamp, UserId,
    WorkflowStage, WorkspaceId,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

/// Persisted authentication and local runner identity for a collaboration server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

    /// Resolves the auth session for `workspace_id`, falling back to the latest saved session.
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
            if let Some(url) = ws_server
                && let Some(sess) = Self::get_by_server(conn, &url)?
            {
                return Ok(Some(sess));
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
        }
        Ok(Self::list_sessions(conn)?.into_iter().next())
    }

    /// Records the registered `runner_id` and `runner_name` on the matching auth session.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn set_runner_identity(
        conn: &Connection,
        server_url: &str,
        runner_id: &RunnerId,
        runner_name: &str,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        conn.execute(
            "UPDATE team_auth_sessions SET runner_id = ?2, runner_name = ?3, updated_at = ?4 WHERE server_url = ?1",
            params![
                server_url.trim_end_matches('/'),
                runner_id.as_ref(),
                runner_name,
                to_sql_i64(now.0)?
            ],
        )?;
        Ok(())
    }
}

/// Buffered per-run event waiting to be flushed by `RunnerAgentWorker` on reconnect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BufferedRunnerReport {
    pub run_id: RunId,
    pub seq: EventSeq,
    pub workspace_id: WorkspaceId,
    pub runner_id: RunnerId,
    pub lease_epoch: LeaseEpoch,
    pub status: Option<RunStatus>,
    pub workflow_stage: Option<WorkflowStage>,
    pub head_sha: Option<String>,
    pub base_sha: Option<String>,
    pub event_type: String,
    pub payload_json: String,
    pub created_at: Timestamp,
}

/// Durable buffer for fenced runner events emitted while offline.
pub struct RunnerBufferStore;

impl RunnerBufferStore {
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

    fn str_to_run_status(s: &str) -> Result<RunStatus, StorageError> {
        match s {
            "queued" => Ok(RunStatus::Queued),
            "running" => Ok(RunStatus::Running),
            "waiting_for_approval" => Ok(RunStatus::WaitingForApproval),
            "cancel_requested" => Ok(RunStatus::CancelRequested),
            "stopped" => Ok(RunStatus::Stopped),
            "completed" => Ok(RunStatus::Completed),
            "failed" => Ok(RunStatus::Failed),
            "interrupted" => Ok(RunStatus::Interrupted),
            other => Err(StorageError::InvalidState(format!(
                "unknown run status `{other}`"
            ))),
        }
    }

    fn row_to_buffered(row: &Row<'_>) -> rusqlite::Result<BufferedRunnerReport> {
        let run_id: String = row.get(0)?;
        let seq = from_sql_i64(row.get::<_, i64>(1)?, 1)?;
        let workspace_id: String = row.get(2)?;
        let runner_id: String = row.get(3)?;
        let lease_epoch = from_sql_i64(row.get::<_, i64>(4)?, 4)?;
        let status_opt: Option<String> = row.get(5)?;
        let wf_opt: Option<String> = row.get(6)?;
        let head_sha: Option<String> = row.get(7)?;
        let base_sha: Option<String> = row.get(8)?;
        let event_type: String = row.get(9)?;
        let payload_json: String = row.get(10)?;
        let created_at = from_sql_i64(row.get::<_, i64>(11)?, 11)?;

        let status = status_opt
            .as_deref()
            .map(Self::str_to_run_status)
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    5,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
        let workflow_stage = wf_opt
            .as_deref()
            .map(str_to_workflow_stage)
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    6,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;

        Ok(BufferedRunnerReport {
            run_id: RunId::from(run_id),
            seq: EventSeq(seq),
            workspace_id: WorkspaceId::from(workspace_id),
            runner_id: RunnerId::from(runner_id),
            lease_epoch: LeaseEpoch(lease_epoch),
            status,
            workflow_stage,
            head_sha,
            base_sha,
            event_type,
            payload_json,
            created_at: Timestamp(created_at),
        })
    }

    /// Buffers all events in `report` inside `runner_buffered_events` in `seq` order.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL or conversion failure (including duplicate `seq`).
    pub fn buffer_report(
        conn: &Connection,
        workspace_id: &WorkspaceId,
        run_id: &RunId,
        report: &RunnerRunReport,
    ) -> Result<(), StorageError> {
        let status_str = report.status.map(Self::run_status_to_str);
        let stage_str = report.workflow_stage.map(workflow_stage_to_str);
        for ev in &report.events {
            conn.execute(
                concat!(
                    "INSERT INTO runner_buffered_events (",
                    "run_id, seq, workspace_id, runner_id, lease_epoch, status, workflow_stage, ",
                    "head_sha, base_sha, event_type, payload_json, created_at, flushed) ",
                    "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0)"
                ),
                params![
                    run_id.as_ref(),
                    to_sql_i64(ev.seq.0)?,
                    workspace_id.as_ref(),
                    report.runner_id.as_ref(),
                    to_sql_i64(report.lease_epoch.0)?,
                    status_str,
                    stage_str,
                    report.head_sha,
                    report.base_sha,
                    ev.event_type,
                    ev.payload_json,
                    to_sql_i64(ev.created_at.0)?,
                ],
            )?;
        }
        Ok(())
    }

    /// Atomically allocates the next `EventSeq` inside `tx` (already holding an `IMMEDIATE` lock)
    /// and inserts a single event from `report`, returning the assigned `EventSeq`.
    ///
    /// # Errors
    /// Returns [`StorageError`] if `report.events` is not length 1 or SQL insertion fails.
    pub fn allocate_and_buffer_in_tx(
        tx: &Connection,
        workspace_id: &WorkspaceId,
        run_id: &RunId,
        report: &RunnerRunReport,
    ) -> Result<EventSeq, StorageError> {
        let [ev] = report.events.as_slice() else {
            return Err(StorageError::InvalidState(
                "allocate_and_buffer_in_tx requires exactly 1 event".to_string(),
            ));
        };
        let seq = Self::next_seq_for_run(tx, run_id)?;
        let mut with_seq = report.clone();
        with_seq.events = vec![runlark_domain::RunnerEventInput {
            seq,
            event_type: ev.event_type.clone(),
            payload_json: ev.payload_json.clone(),
            created_at: ev.created_at,
        }];
        Self::buffer_report(tx, workspace_id, run_id, &with_seq)?;
        Ok(seq)
    }

    /// Lists unflushed runner events ordered strictly by `(run_id ASC, seq ASC)`.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn list_unflushed(
        conn: &Connection,
        workspace_id: Option<&WorkspaceId>,
    ) -> Result<Vec<BufferedRunnerReport>, StorageError> {
        let cols = concat!(
            "run_id, seq, workspace_id, runner_id, lease_epoch, status, workflow_stage, ",
            "head_sha, base_sha, event_type, payload_json, created_at"
        );
        if let Some(wid) = workspace_id {
            let sql = format!(
                "SELECT {cols} FROM runner_buffered_events WHERE flushed = 0 AND workspace_id = ?1 ORDER BY run_id ASC, seq ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![wid.as_ref()], Self::row_to_buffered)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        } else {
            let sql = format!(
                "SELECT {cols} FROM runner_buffered_events WHERE flushed = 0 ORDER BY run_id ASC, seq ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], Self::row_to_buffered)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        }
    }

    /// Lists unflushed runner events for a single `(workspace_id, run_id)` ordered by `seq ASC`.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn list_unflushed_for_run(
        conn: &Connection,
        workspace_id: &WorkspaceId,
        run_id: &RunId,
    ) -> Result<Vec<BufferedRunnerReport>, StorageError> {
        let sql = concat!(
            "SELECT run_id, seq, workspace_id, runner_id, lease_epoch, status, workflow_stage, ",
            "head_sha, base_sha, event_type, payload_json, created_at ",
            "FROM runner_buffered_events WHERE flushed = 0 AND workspace_id = ?1 AND run_id = ?2 ",
            "ORDER BY seq ASC"
        );
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(
            params![workspace_id.as_ref(), run_id.as_ref()],
            Self::row_to_buffered,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Marks buffered events in the inclusive `[min_seq, max_seq]` range for `run_id` as flushed.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn mark_flushed_range(
        conn: &Connection,
        run_id: &RunId,
        min_seq: EventSeq,
        max_seq: EventSeq,
    ) -> Result<(), StorageError> {
        conn.execute(
            "UPDATE runner_buffered_events SET flushed = 1 WHERE run_id = ?1 AND seq >= ?2 AND seq <= ?3",
            params![
                run_id.as_ref(),
                to_sql_i64(min_seq.0)?,
                to_sql_i64(max_seq.0)?
            ],
        )?;
        Ok(())
    }

    /// Marks buffered events up to `max_seq` for `run_id` as flushed.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn mark_flushed_up_to(
        conn: &Connection,
        run_id: &RunId,
        max_seq: EventSeq,
    ) -> Result<(), StorageError> {
        conn.execute(
            "UPDATE runner_buffered_events SET flushed = 1 WHERE run_id = ?1 AND seq <= ?2",
            params![run_id.as_ref(), to_sql_i64(max_seq.0)?],
        )?;
        Ok(())
    }

    /// Returns the next monotonic `EventSeq` for `run_id` across buffered events.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn next_seq_for_run(conn: &Connection, run_id: &RunId) -> Result<EventSeq, StorageError> {
        let max_seq: Option<i64> = conn.query_row(
            "SELECT MAX(seq) FROM runner_buffered_events WHERE run_id = ?1",
            params![run_id.as_ref()],
            |row| row.get(0),
        )?;
        let next = match max_seq {
            Some(v) => from_sql_i64(v, 0)?.saturating_add(1),
            None => 1,
        };
        Ok(EventSeq(next))
    }
}
