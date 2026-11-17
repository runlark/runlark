use crate::connection::to_sql_i64;
use crate::error::StorageError;
use runlark_domain::{CommandId, RunId, RunStatus, SessionId, Timestamp};
use rusqlite::{Connection, params};

/// State of inflight runs and outbox commands detected during startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InflightState {
    pub active_runs: Vec<(RunId, SessionId, RunStatus)>,
    pub unacked_commands: Vec<(CommandId, SessionId, RunId, String)>,
}

pub struct ReconciliationEngine;

impl ReconciliationEngine {
    /// Detects inflight runs and outbox commands on daemon startup.
    ///
    /// # Errors
    /// Returns `StorageError` if SQL query fails.
    pub fn scan_inflight(conn: &Connection) -> Result<InflightState, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT run_id, session_id, status FROM runs ",
            "WHERE status IN ('running', 'cancel_requested', 'queued')"
        ))?;
        let active_runs = stmt
            .query_map([], |row| {
                let r_id: String = row.get(0)?;
                let s_id: String = row.get(1)?;
                let status_str: String = row.get(2)?;
                let status = match status_str.as_str() {
                    "running" => RunStatus::Running,
                    "cancel_requested" => RunStatus::CancelRequested,
                    _ => RunStatus::Queued,
                };
                Ok((RunId::from(r_id), SessionId::from(s_id), status))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut cmd_stmt = conn.prepare(concat!(
            "SELECT command_id, session_id, run_id, status FROM commands ",
            "WHERE status IN ('pending', 'dispatched')"
        ))?;
        let unacked_commands = cmd_stmt
            .query_map([], |row| {
                let c_id: String = row.get(0)?;
                let s_id: String = row.get(1)?;
                let r_id: String = row.get(2)?;
                let status: String = row.get(3)?;
                Ok((
                    CommandId::from(c_id),
                    SessionId::from(s_id),
                    RunId::from(r_id),
                    status,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(InflightState {
            active_runs,
            unacked_commands,
        })
    }

    /// Marks an inflight run (`running`, `cancel_requested`, or `queued`) as `Interrupted`,
    /// leaving terminal runs untouched.
    ///
    /// # Errors
    /// Returns `StorageError` if SQL update fails.
    pub fn mark_interrupted(
        conn: &Connection,
        run_id: &RunId,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        conn.execute(
            concat!(
                "UPDATE runs SET status = 'interrupted', updated_at = ?2 ",
                "WHERE run_id = ?1 AND status IN ('running', 'cancel_requested', 'queued')"
            ),
            params![run_id.as_ref(), to_sql_i64(now.0)],
        )?;
        Ok(())
    }

    /// Performs automatic startup reconciliation for all active runs.
    ///
    /// # Errors
    /// Returns `StorageError` if database query or updates fail.
    pub fn reconcile_on_startup(
        conn: &mut Connection,
        now: Timestamp,
    ) -> Result<InflightState, StorageError> {
        let inflight = Self::scan_inflight(conn)?;

        let tx = conn.transaction()?;
        for (run_id, _, _) in &inflight.active_runs {
            Self::mark_interrupted(&tx, run_id, now)?;
        }
        tx.commit()?;

        Ok(inflight)
    }
}
