use crate::connection::{from_sql_i64, to_sql_i64};
use crate::enum_codec::{str_to_workflow_stage, workflow_stage_to_str};
use crate::error::StorageError;
use runlark_domain::{Attempt, AttemptId, AttemptOutcome, CostSummary, RunId, Timestamp};
use rusqlite::{Connection, OptionalExtension, Row, params};

pub struct AttemptStore;

impl AttemptStore {
    fn row_to_attempt(row: &Row<'_>) -> rusqlite::Result<Attempt> {
        let id: String = row.get(0)?;
        let run_id: String = row.get(1)?;
        let attempt_no_raw: i64 = row.get(2)?;
        let wf_stage_str: String = row.get(3)?;
        let started = from_sql_i64(row.get::<_, i64>(4)?, 4)?;
        let ended_opt: Option<i64> = row.get(5)?;
        let outcome_json: String = row.get(6)?;
        let cost_opt: Option<i64> = row.get(7)?;
        let unknown_cost_reason: Option<String> = row.get(8)?;
        let repair_reason: Option<String> = row.get(9)?;

        let attempt_no = u32::try_from(attempt_no_raw).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Integer,
                Box::new(e),
            )
        })?;
        let workflow_stage = str_to_workflow_stage(&wf_stage_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let ended_at = ended_opt
            .map(|v| from_sql_i64(v, 5))
            .transpose()?
            .map(Timestamp);
        let outcome = serde_json::from_str::<AttemptOutcome>(&outcome_json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let cost_micro_usd = cost_opt.map(|v| from_sql_i64(v, 7)).transpose()?;

        Ok(Attempt {
            id: AttemptId::from(id),
            run_id: RunId::from(run_id),
            attempt_no,
            workflow_stage,
            started_at: Timestamp(started),
            ended_at,
            outcome,
            cost_micro_usd,
            unknown_cost_reason,
            repair_reason,
        })
    }

    /// Inserts or replaces an [`Attempt`] record, preserving SQL `NULL` for unknown cost.
    ///
    /// # Errors
    /// Returns [`StorageError`] if serialization, integer conversion, or SQL insertion fails.
    pub fn save_attempt(conn: &Connection, attempt: &Attempt) -> Result<(), StorageError> {
        let outcome_json = serde_json::to_string(&attempt.outcome)?;
        let ended_sql = attempt.ended_at.map(|t| to_sql_i64(t.0)).transpose()?;
        let cost_sql = attempt.cost_micro_usd.map(to_sql_i64).transpose()?;
        conn.execute(
            concat!(
                "INSERT INTO attempts (",
                "attempt_id, run_id, attempt_no, workflow_stage, started_at, ended_at, ",
                "outcome_json, cost_micro_usd, unknown_cost_reason, repair_reason) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) ",
                "ON CONFLICT(attempt_id) DO UPDATE SET ",
                "ended_at = excluded.ended_at, ",
                "outcome_json = excluded.outcome_json, ",
                "cost_micro_usd = excluded.cost_micro_usd, ",
                "unknown_cost_reason = excluded.unknown_cost_reason, ",
                "repair_reason = excluded.repair_reason"
            ),
            params![
                attempt.id.as_ref(),
                attempt.run_id.as_ref(),
                i64::from(attempt.attempt_no),
                workflow_stage_to_str(attempt.workflow_stage),
                to_sql_i64(attempt.started_at.0)?,
                ended_sql,
                outcome_json,
                cost_sql,
                attempt.unknown_cost_reason,
                attempt.repair_reason,
            ],
        )?;
        Ok(())
    }

    /// Fetches a single [`Attempt`] by ID.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn get_attempt(
        conn: &Connection,
        attempt_id: &AttemptId,
    ) -> Result<Option<Attempt>, StorageError> {
        conn.query_row(
            concat!(
                "SELECT attempt_id, run_id, attempt_no, workflow_stage, started_at, ended_at, ",
                "outcome_json, cost_micro_usd, unknown_cost_reason, repair_reason ",
                "FROM attempts WHERE attempt_id = ?1"
            ),
            params![attempt_id.as_ref()],
            Self::row_to_attempt,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Lists all attempts for a run ordered by `attempt_no` ascending.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn list_attempts_for_run(
        conn: &Connection,
        run_id: &RunId,
    ) -> Result<Vec<Attempt>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT attempt_id, run_id, attempt_no, workflow_stage, started_at, ended_at, ",
            "outcome_json, cost_micro_usd, unknown_cost_reason, repair_reason ",
            "FROM attempts WHERE run_id = ?1 ORDER BY attempt_no ASC"
        ))?;
        let rows = stmt.query_map(params![run_id.as_ref()], Self::row_to_attempt)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Recomputes an explicit [`CostSummary`] for `run_id` from persisted attempts,
    /// preserving any `NULL` cost as `unknown_attempts += 1` (never folded to `0`).
    ///
    /// # Errors
    /// Returns [`StorageError`] if querying attempts fails.
    pub fn cost_summary_for_run(
        conn: &Connection,
        run_id: &RunId,
        held_micro_usd: u64,
    ) -> Result<CostSummary, StorageError> {
        let attempts = Self::list_attempts_for_run(conn, run_id)?;
        Ok(CostSummary::from_attempts(&attempts, held_micro_usd))
    }

    /// Marks any `InProgress` attempts on `run_id` as `AttemptOutcome::Unknown` with `NULL`
    /// cost (`cost_micro_usd = None`) when recovering from an unexpected daemon restart.
    ///
    /// # Errors
    /// Returns [`StorageError`] if querying or updating attempts fails.
    pub fn mark_in_progress_unknown_on_restart(
        conn: &Connection,
        run_id: &RunId,
        reason: &str,
        now: Timestamp,
    ) -> Result<u64, StorageError> {
        let attempts = Self::list_attempts_for_run(conn, run_id)?;
        let mut updated = 0_u64;
        for mut attempt in attempts {
            if matches!(attempt.outcome, AttemptOutcome::InProgress) {
                attempt.outcome = AttemptOutcome::Unknown {
                    reason: reason.to_string(),
                };
                attempt.ended_at = Some(now);
                attempt.cost_micro_usd = None;
                attempt.unknown_cost_reason = Some(reason.to_string());
                Self::save_attempt(conn, &attempt)?;
                updated = updated.saturating_add(1);
            }
        }
        Ok(updated)
    }
}
