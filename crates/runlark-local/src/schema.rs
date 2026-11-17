use crate::error::StorageError;
use rusqlite::Connection;

const INITIAL_SCHEMA: &str = include_str!("../migrations/0001_initial.sql");

/// Initializes the database schema, applying pending migrations.
///
/// # Errors
/// Returns `StorageError` if DDL execution fails.
pub fn initialize_schema(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(INITIAL_SCHEMA)?;
    Ok(())
}
