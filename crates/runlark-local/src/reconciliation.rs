use crate::attempt_store::AttemptStore;
use crate::cas_store::ArtifactCasStore;
use crate::connection::to_sql_i64;
use crate::error::StorageError;
use crate::event_store::EventStore;
use crate::question_store::QuestionStore;
use crate::session_store::SessionStore;
use runlark_domain::{
    ArtifactDigest, CommandId, DomainEvent, Question, RunId, RunStatus, SessionId, TaskId,
    Timestamp,
};
use rusqlite::{Connection, TransactionBehavior, params};

/// State of inflight runs, preserved queues/approvals, and outbox commands detected on startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InflightState {
    pub active_runs: Vec<(RunId, SessionId, RunStatus)>,
    pub queued_runs: Vec<(RunId, SessionId, TaskId)>,
    pub waiting_runs: Vec<(RunId, SessionId)>,
    pub pending_questions: Vec<Question>,
    pub unacked_commands: Vec<(CommandId, SessionId, RunId, String)>,
    pub interrupted_attempts: u64,
}

pub struct ReconciliationEngine;

impl ReconciliationEngine {
    /// Detects inflight runs, preserved `waiting_for_approval` runs/questions, A1 `queued` runs,
    /// and unacked outbox commands on daemon startup.
    ///
    /// - `waiting_for_approval` is preserved because human approvals survive daemon restarts.
    /// - Task-backed `queued` runs (`task_id IS NOT NULL`) are preserved in `queued_runs` for
    ///   2-slot scheduler admission after restart; legacy A0 `queued` runs (`task_id IS NULL`)
    ///   without a task queue record remain in `active_runs` to be reconciled as `Interrupted`.
    ///
    /// # Errors
    /// Returns [`StorageError`] if SQL queries fail.
    pub fn scan_inflight(conn: &Connection) -> Result<InflightState, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT run_id, session_id, status FROM runs ",
            "WHERE status IN ('running', 'cancel_requested') ",
            "OR (status = 'queued' AND task_id IS NULL)"
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

        let mut q_stmt = conn.prepare(concat!(
            "SELECT run_id, session_id, task_id FROM runs ",
            "WHERE status = 'queued' AND task_id IS NOT NULL ",
            "ORDER BY created_at ASC, rowid ASC"
        ))?;
        let queued_runs = q_stmt
            .query_map([], |row| {
                let r_id: String = row.get(0)?;
                let s_id: String = row.get(1)?;
                let t_id: String = row.get(2)?;
                Ok((RunId::from(r_id), SessionId::from(s_id), TaskId::from(t_id)))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut wait_stmt = conn.prepare(concat!(
            "SELECT run_id, session_id FROM runs ",
            "WHERE status = 'waiting_for_approval'"
        ))?;
        let waiting_runs = wait_stmt
            .query_map([], |row| {
                let r_id: String = row.get(0)?;
                let s_id: String = row.get(1)?;
                Ok((RunId::from(r_id), SessionId::from(s_id)))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let pending_questions = QuestionStore::list_pending(conn)?;

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
            queued_runs,
            waiting_runs,
            pending_questions,
            unacked_commands,
            interrupted_attempts: 0,
        })
    }

    /// Marks an inflight run as `Interrupted`, leaving `waiting_for_approval`, A1 task-backed
    /// `queued` runs, and terminal runs untouched.
    ///
    /// # Errors
    /// Returns [`StorageError`] if SQL update fails.
    pub fn mark_interrupted(
        conn: &Connection,
        run_id: &RunId,
        now: Timestamp,
    ) -> Result<bool, StorageError> {
        let affected = conn.execute(
            concat!(
                "UPDATE runs SET status = 'interrupted', updated_at = ?2 ",
                "WHERE run_id = ?1 AND (status IN ('running', 'cancel_requested') ",
                "OR (status = 'queued' AND task_id IS NULL))"
            ),
            params![run_id.as_ref(), to_sql_i64(now.0)?],
        )?;
        Ok(affected > 0)
    }

    /// Performs automatic startup reconciliation for all active runs, recording `RunInterrupted`
    /// facts, converting any `InProgress` attempts to `AttemptOutcome::Unknown` (preserving
    /// unknown cost rather than coalescing to zero), and preserving `waiting_for_approval` and
    /// task-backed `queued` runs.
    ///
    /// # Errors
    /// Returns [`StorageError`] if database queries or updates fail.
    pub fn reconcile_on_startup(
        conn: &mut Connection,
        now: Timestamp,
    ) -> Result<InflightState, StorageError> {
        let mut inflight = Self::scan_inflight(conn)?;

        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut interrupted_attempts = 0_u64;
        for (run_id, _, _) in &inflight.active_runs {
            if Self::mark_interrupted(&tx, run_id, now)? {
                let changed = AttemptStore::mark_in_progress_unknown_on_restart(
                    &tx,
                    run_id,
                    "daemon restarted during attempt before cost settlement",
                    now,
                )?;
                interrupted_attempts = interrupted_attempts.saturating_add(changed);
                if changed > 0
                    && let Some(mut run) = SessionStore::get_run(&tx, run_id)?
                {
                    run.cost_summary = AttemptStore::cost_summary_for_run(
                        &tx,
                        run_id,
                        run.cost_summary.held_micro_usd,
                    )?;
                    SessionStore::save_run(&tx, &run)?;
                }
                let event = DomainEvent::RunInterrupted {
                    run_id: run_id.clone(),
                    reason: "daemon restarted while run was inflight".to_string(),
                    timestamp: now,
                };
                EventStore::append_domain_event(&tx, run_id, &event, now)?;
            }
        }
        tx.commit()?;

        inflight.interrupted_attempts = interrupted_attempts;
        Ok(inflight)
    }

    /// Verifies that every indexed artifact in `SQLite` exists in `cas` and passes SHA-256
    /// verification, returning any missing or corrupt digests.
    ///
    /// # Errors
    /// Returns [`StorageError`] if querying `artifacts` fails.
    pub fn verify_cas_integrity(
        conn: &Connection,
        cas: &ArtifactCasStore,
    ) -> Result<Vec<ArtifactDigest>, StorageError> {
        let mut stmt =
            conn.prepare("SELECT digest_sha256 FROM artifacts ORDER BY created_at ASC")?;
        let digests: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;

        let mut bad = Vec::new();
        for raw in digests {
            let digest = ArtifactDigest::parse(raw)?;
            if cas.get_bytes(&digest).is_err() {
                bad.push(digest);
            }
        }
        Ok(bad)
    }
}
