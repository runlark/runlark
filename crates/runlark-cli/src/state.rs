use runlark_local::{Connection, StorageError, open_sqlite};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
pub struct AppState {
    pub db_path: PathBuf,
    pub start_time: Arc<Instant>,
}

impl AppState {
    #[must_use]
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            db_path,
            start_time: Arc::new(Instant::now()),
        }
    }

    /// Opens a new connection to the local database.
    ///
    /// # Errors
    /// Returns `StorageError` if opening the `SQLite` database fails.
    pub fn get_connection(&self) -> Result<Connection, StorageError> {
        open_sqlite(&self.db_path)
    }
}
