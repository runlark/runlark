use runlark_domain::{CommandId, RunId, SessionId};
use thiserror::Error;

/// Storage and persistence errors for local `SQLite` operations.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Command payload conflict: command {0} exists with differing payload")]
    CommandPayloadConflict(CommandId),

    #[error("Session not found: {0}")]
    SessionNotFound(SessionId),

    #[error("Run not found: {0}")]
    RunNotFound(RunId),

    #[error("Invalid storage state: {0}")]
    InvalidState(String),
}
