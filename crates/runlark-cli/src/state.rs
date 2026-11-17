use runlark_local::{Connection, StorageError, StoredEvent, open_sqlite};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{Notify, broadcast, watch};

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
    pub start_time: Arc<Instant>,
    pub outbox_notify: Arc<Notify>,
    pub dsh_notify: Arc<Notify>,
    pub event_tx: broadcast::Sender<StreamEventNotification>,
    pub shutdown_tx: watch::Sender<bool>,
}

impl AppState {
    #[must_use]
    pub fn new(db_path: PathBuf) -> Self {
        let (event_tx, _) = broadcast::channel(256);
        let (shutdown_tx, _) = watch::channel(false);
        Self {
            db_path,
            start_time: Arc::new(Instant::now()),
            outbox_notify: Arc::new(Notify::new()),
            dsh_notify: Arc::new(Notify::new()),
            event_tx,
            shutdown_tx,
        }
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
