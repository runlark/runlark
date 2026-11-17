use crate::verification::VerificationService;
use runlark_local::{ArtifactCasStore, Connection, StorageError, StoredEvent, open_sqlite};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, Notify, broadcast, watch};

/// Notification emitted whenever new run events are persisted to `SQLite`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamEventNotification {
    pub session_id: String,
    pub run_id: String,
    pub seq: u64,
}

#[derive(Clone)]
pub struct AppState {
    pub db_path: PathBuf,
    pub data_dir: PathBuf,
    pub start_time: Arc<Instant>,
    pub outbox_notify: Arc<Notify>,
    pub dsh_notify: Arc<Notify>,
    pub scheduler_notify: Arc<Notify>,
    pub scheduler_admit_lock: Arc<Mutex<()>>,
    pub event_tx: broadcast::Sender<StreamEventNotification>,
    pub shutdown_tx: watch::Sender<bool>,
    pub verification: VerificationService,
    pub project_check_command: Option<String>,
}

impl AppState {
    /// Creates application state with an in-memory WASM verification registry.
    ///
    /// # Panics
    /// Panics only if initializing the Wasmtime engine or embedded canonical WIT contract fails.
    #[must_use]
    pub fn new(db_path: PathBuf) -> Self {
        let verification = VerificationService::in_memory()
            .expect("embedded WIT contract and Wasmtime engine must initialize");
        Self::with_verification(db_path, verification)
    }

    /// Creates application state with an explicit [`VerificationService`].
    #[must_use]
    pub fn with_verification(db_path: PathBuf, verification: VerificationService) -> Self {
        let data_dir = db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let (event_tx, _) = broadcast::channel(256);
        let (shutdown_tx, _) = watch::channel(false);
        let project_check_command = std::env::var("RUNLARK_PROJECT_CHECK")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        Self {
            db_path,
            data_dir,
            start_time: Arc::new(Instant::now()),
            outbox_notify: Arc::new(Notify::new()),
            dsh_notify: Arc::new(Notify::new()),
            scheduler_notify: Arc::new(Notify::new()),
            scheduler_admit_lock: Arc::new(Mutex::new(())),
            event_tx,
            shutdown_tx,
            verification,
            project_check_command,
        }
    }

    /// Returns a copy of `AppState` configured with an explicit shell project check command.
    #[must_use]
    pub fn with_project_check(mut self, command: Option<String>) -> Self {
        self.project_check_command = command;
        self
    }

    /// Returns the content-addressed artifact store rooted under `<data_dir>/cas`.
    #[must_use]
    pub fn cas_store(&self) -> ArtifactCasStore {
        ArtifactCasStore::from_data_dir(&self.data_dir)
    }

    /// Wakes the 2-slot scheduler to admit eligible `Queued` runs.
    pub fn notify_scheduler(&self) {
        self.scheduler_notify.notify_one();
    }

    /// Signals all in-flight long-poll listeners that the daemon is shutting down.
    pub fn signal_shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Subscribes to daemon shutdown notifications.
    #[must_use]
    pub fn subscribe_shutdown(&self) -> watch::Receiver<bool> {
        self.shutdown_tx.subscribe()
    }

    /// Opens a new connection to the local database.
    ///
    /// # Errors
    /// Returns `StorageError` if opening the `SQLite` database fails.
    pub fn get_connection(&self) -> Result<Connection, StorageError> {
        open_sqlite(&self.db_path)
    }

    /// Wakes the background outbox dispatcher to process newly persisted commands.
    pub fn notify_outbox(&self) {
        self.outbox_notify.notify_one();
    }

    /// Wakes the DSH WebSocket subscriber to sync newly created/activated sessions.
    pub fn notify_dsh_subscriber(&self) {
        self.dsh_notify.notify_one();
    }

    /// Subscribes to live event persistence notifications.
    #[must_use]
    pub fn subscribe_events(&self) -> broadcast::Receiver<StreamEventNotification> {
        self.event_tx.subscribe()
    }

    /// Broadcasts newly persisted domain events to waiting stream listeners.
    pub fn publish_events(&self, session_id: &str, events: &[StoredEvent]) {
        for event in events {
            let _ = self.event_tx.send(StreamEventNotification {
                session_id: session_id.to_string(),
                run_id: event.run_id.0.clone(),
                seq: event.seq.0,
            });
        }
    }

    /// Returns the current UNIX timestamp in milliseconds.
    #[must_use]
    pub fn current_timestamp_ms() -> u64 {
        u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        )
        .unwrap_or(u64::MAX)
    }
}
