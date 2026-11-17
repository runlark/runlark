use crate::connection::to_sql_i64;
use crate::error::StorageError;
use rusqlite::{Connection, OptionalExtension, params};
use std::time::{SystemTime, UNIX_EPOCH};

const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/0001_initial.sql")),
    (2, include_str!("../migrations/0002_questions.sql")),
    (3, include_str!("../migrations/0003_dsh_cursors.sql")),
];

fn current_timestamp_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

/// Applies all `SQLite` schema migrations in version order, recording applied versions
/// in `schema_migrations` for idempotency.
///
/// # Errors
/// Returns `StorageError` if DDL execution or migration bookkeeping fails.
pub fn initialize_schema(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY NOT NULL,
            applied_at INTEGER NOT NULL
        );",
    )?;

    let now = to_sql_i64(current_timestamp_ms())?;

    for &(version, sql) in MIGRATIONS {
        let already_applied: Option<i64> = conn
            .query_row(
                "SELECT version FROM schema_migrations WHERE version = ?1",
                params![version],
                |row| row.get(0),
            )
            .optional()?;

        if already_applied.is_none() {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![version, now],
            )?;
            tx.commit()?;
        }
    }

    Ok(())
}
