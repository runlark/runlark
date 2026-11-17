use crate::connection::{from_sql_i64, to_sql_i64};
use crate::enum_codec::{decision_kind_to_str, str_to_decision_kind};
use crate::error::StorageError;
use runlark_domain::{
    ArtifactDigest, Decision, DecisionId, DecisionKind, PolicyDigest, RunId, TaskId, Timestamp,
};
use rusqlite::{Connection, OptionalExtension, Row, params};

pub struct DecisionStore;

impl DecisionStore {
    fn row_to_decision(row: &Row<'_>) -> rusqlite::Result<Decision> {
        let id: String = row.get(0)?;
        let task_id: String = row.get(1)?;
        let run_id: String = row.get(2)?;
        let kind_str: String = row.get(3)?;
        let actor: String = row.get(4)?;
        let plan_ver_raw: i64 = row.get(5)?;
        let head_sha: String = row.get(6)?;
        let base_sha: String = row.get(7)?;
        let policy_digest: String = row.get(8)?;
        let evidence_str: String = row.get(9)?;
        let comment: Option<String> = row.get(10)?;
        let created = from_sql_i64(row.get::<_, i64>(11)?, 11)?;

        let kind = str_to_decision_kind(&kind_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let plan_version = u32::try_from(plan_ver_raw).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Integer,
                Box::new(e),
            )
        })?;
        let evidence_digest = ArtifactDigest::parse(evidence_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(e))
        })?;

        Ok(Decision {
            id: DecisionId::from(id),
            task_id: TaskId::from(task_id),
            run_id: RunId::from(run_id),
            kind,
            actor,
            plan_version,
            head_sha,
            base_sha,
            policy_digest: PolicyDigest::from(policy_digest),
            evidence_digest,
            comment,
            created_at: Timestamp(created),
        })
    }

    /// Persists an immutable human [`Decision`] into `SQLite`.
    ///
    /// # Errors
    /// Returns [`StorageError`] if SQL insertion fails.
    pub fn record_decision(conn: &Connection, decision: &Decision) -> Result<(), StorageError> {
        conn.execute(
            concat!(
                "INSERT INTO decisions (",
                "decision_id, task_id, run_id, kind, actor, plan_version, head_sha, base_sha, ",
                "policy_digest, evidence_digest, comment, created_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
            ),
            params![
                decision.id.as_ref(),
                decision.task_id.as_ref(),
                decision.run_id.as_ref(),
                decision_kind_to_str(decision.kind),
                decision.actor,
                i64::from(decision.plan_version),
                decision.head_sha,
                decision.base_sha,
                decision.policy_digest.as_ref(),
                decision.evidence_digest.as_str(),
                decision.comment,
                to_sql_i64(decision.created_at.0)?,
            ],
        )?;
        Ok(())
    }

    /// Fetches a [`Decision`] by ID.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn get_decision(
        conn: &Connection,
        decision_id: &DecisionId,
    ) -> Result<Option<Decision>, StorageError> {
        conn.query_row(
            concat!(
                "SELECT decision_id, task_id, run_id, kind, actor, plan_version, head_sha, ",
                "base_sha, policy_digest, evidence_digest, comment, created_at ",
                "FROM decisions WHERE decision_id = ?1"
            ),
            params![decision_id.as_ref()],
            Self::row_to_decision,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Lists all decisions for a task in chronological order.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn list_decisions_for_task(
        conn: &Connection,
        task_id: &TaskId,
    ) -> Result<Vec<Decision>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT decision_id, task_id, run_id, kind, actor, plan_version, head_sha, ",
            "base_sha, policy_digest, evidence_digest, comment, created_at ",
            "FROM decisions WHERE task_id = ?1 ORDER BY created_at ASC, rowid ASC"
        ))?;
        let rows = stmt.query_map(params![task_id.as_ref()], Self::row_to_decision)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Lists all decisions for a run in chronological order.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn list_decisions_for_run(
        conn: &Connection,
        run_id: &RunId,
    ) -> Result<Vec<Decision>, StorageError> {
        let mut stmt = conn.prepare(concat!(
            "SELECT decision_id, task_id, run_id, kind, actor, plan_version, head_sha, ",
            "base_sha, policy_digest, evidence_digest, comment, created_at ",
            "FROM decisions WHERE run_id = ?1 ORDER BY created_at ASC, rowid ASC"
        ))?;
        let rows = stmt.query_map(params![run_id.as_ref()], Self::row_to_decision)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Returns the most recent `DecisionKind::Accept` decision for `run_id`, if any.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn latest_accept_for_run(
        conn: &Connection,
        run_id: &RunId,
    ) -> Result<Option<Decision>, StorageError> {
        conn.query_row(
            concat!(
                "SELECT decision_id, task_id, run_id, kind, actor, plan_version, head_sha, ",
                "base_sha, policy_digest, evidence_digest, comment, created_at ",
                "FROM decisions WHERE run_id = ?1 AND kind = ?2 ",
                "ORDER BY created_at DESC, rowid DESC LIMIT 1"
            ),
            params![run_id.as_ref(), decision_kind_to_str(DecisionKind::Accept)],
            Self::row_to_decision,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Returns `true` if `task_id` has a recorded `DecisionKind::Accept`.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn has_accept_for_task(conn: &Connection, task_id: &TaskId) -> Result<bool, StorageError> {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM decisions WHERE task_id = ?1 AND kind = ?2",
            params![task_id.as_ref(), decision_kind_to_str(DecisionKind::Accept)],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}
