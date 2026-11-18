use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Runtime configuration for `runlark-server` (shared across `SaaS` and self-host deployments).
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub postgres_url: String,
    pub cas_root: PathBuf,
    pub runner_heartbeat_timeout_ms: u64,
}

impl ServerConfig {
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
