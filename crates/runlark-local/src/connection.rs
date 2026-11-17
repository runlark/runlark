use crate::error::StorageError;
use rusqlite::Connection;
use std::path::Path;

#[must_use]
pub fn to_sql_i64(val: u64) -> i64 {
    i64::try_from(val).unwrap_or(i64::MAX)
}

#[must_use]
pub fn from_sql_i64(val: i64) -> u64 {
    u64::try_from(val).unwrap_or_default()
}

/// Configures standard pragmas on a `SQLite` connection for durable concurrency.
///
/// # Errors
/// Returns `StorageError` if pragma execution fails.
pub fn configure_connection(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(concat!(
        "PRAGMA journal_mode = WAL;",
        "PRAGMA synchronous = NORMAL;",
        "PRAGMA foreign_keys = ON;",
        "PRAGMA busy_timeout = 5000;"
    ))?;
    Ok(())
}

/// Opens a `SQLite` database at the specified path with standard concurrency pragmas.
///
/// # Errors
/// Returns `StorageError` if connection opening or pragma setup fails.
pub fn open_sqlite<P: AsRef<Path>>(path: P) -> Result<Connection, StorageError> {
    let conn = Connection::open(path)?;
    configure_connection(&conn)?;
    Ok(conn)
}

/// Opens an in-memory `SQLite` database for ephemeral test suites.
///
/// # Errors
/// Returns `StorageError` if in-memory connection initialization fails.
pub fn open_in_memory() -> Result<Connection, StorageError> {
    let conn = Connection::open_in_memory()?;
    configure_connection(&conn)?;
    Ok(conn)
}
