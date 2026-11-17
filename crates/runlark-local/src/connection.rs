use crate::error::StorageError;
use rusqlite::{Connection, TransactionBehavior};
use std::path::Path;

/// Converts an unsigned integer to `SQLite` `i64`, rejecting values above `i64::MAX`.
///
/// # Errors
/// Returns `StorageError::InvalidState` if `val` exceeds `i64::MAX`.
pub fn to_sql_i64(val: u64) -> Result<i64, StorageError> {
    i64::try_from(val).map_err(|_| {
        StorageError::InvalidState(format!("integer value {val} exceeds SQLite i64 range"))
    })
}

/// Converts a non-negative `SQLite` `i64` column value back to `u64`.
///
/// # Errors
/// Returns `rusqlite::Error::IntegralValueOutOfRange` if `val` is negative.
pub fn from_sql_i64(val: i64, column: usize) -> rusqlite::Result<u64> {
    u64::try_from(val).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, val))
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

/// Runs `f` inside a single `TransactionBehavior::Immediate` `SQLite` transaction and commits it.
///
/// # Errors
/// Returns `E` (and rolls back the transaction on drop) if starting the transaction,
/// running `f`, or committing fails.
pub fn with_immediate_tx<T, E, F>(conn: &mut Connection, f: F) -> Result<T, E>
where
    E: From<StorageError>,
    F: FnOnce(&Connection) -> Result<T, E>,
{
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(StorageError::from)?;
    let result = f(&tx)?;
    tx.commit().map_err(StorageError::from)?;
    Ok(result)
}
