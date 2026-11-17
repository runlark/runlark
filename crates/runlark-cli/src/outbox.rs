use crate::dsh_subscriber::authenticate_dsh_cookie;
use crate::worktree::WorktreeManager;
use reqwest::Client;
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, COOKIE, ORIGIN};
use reqwest::redirect::Policy;
use runlark_domain::{SessionCommand, Timestamp};
use runlark_local::{CommandStore, PersistedCommand, SessionStore, StorageError, open_sqlite};
use serde::Serialize;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Notify, RwLock, watch};

/// Outcome of dispatching a persisted outbox command to the external DSH runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectOutcome {
    /// External runtime confirmed receipt/execution.
    Acked,
    /// Dispatch failed cleanly before confirmation; safe to retry later.
    Failed { error: String },
    /// Delivery or execution outcome is unknown (e.g., timeout or gateway 502/504).
    /// Command remains in `dispatched` state without creating duplicate runs or IDs until reconciled.
    Unknown { reason: String },
}

/// Context passed to an [`OutboxEffector`] for each command dispatch attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchContext {
    pub command: PersistedCommand,
    pub parsed: SessionCommand,
    pub worktree_path: Option<PathBuf>,
}

/// Summary statistics from a single outbox dispatch pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DispatchStats {
    pub attempted: usize,
    pub acked: usize,
    pub failed: usize,
    pub unknown: usize,
}

/// Configuration for [`OutboxDispatcher`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxConfig {
    pub poll_interval: Duration,
    pub retry_backoff_ms: u64,
    pub cleanup_worktree_on_archive: bool,
}

impl Default for OutboxConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(2),
            retry_backoff_ms: 1_000,
            cleanup_worktree_on_archive: false,
        }
    }
}

/// Pluggable external effector executed by [`OutboxDispatcher`] after intent persistence.
pub trait OutboxEffector: Send + Sync {
    fn execute<'a>(
        &'a self,
        ctx: &'a DispatchContext,
    ) -> Pin<Box<dyn Future<Output = EffectOutcome> + Send + 'a>>;
}

/// Default local effector when Runlark operates without an external push DSH bridge URL
/// (e.g., plugin-driven interceptor mode or standalone daemon).
#[derive(Debug, Default)]
pub struct LocalBridgeEffector;

impl OutboxEffector for LocalBridgeEffector {
    fn execute<'a>(
        &'a self,
        _ctx: &'a DispatchContext,
    ) -> Pin<Box<dyn Future<Output = EffectOutcome> + Send + 'a>> {
        Box::pin(async { EffectOutcome::Acked })
    }
}

#[derive(Serialize)]
struct HttpDispatchPayload<'a> {
    command_id: &'a str,
    session_id: &'a str,
    run_id: &'a str,
    command: &'a SessionCommand,
    worktree_path: Option<String>,
}

/// HTTP DSH bridge effector forwarding persisted commands to a DSH host bridge endpoint.
pub struct HttpDshEffector {
    client: Client,
    origin: String,
    endpoint: String,
    token: Option<String>,
    cookie: RwLock<Option<String>>,
}

impl HttpDshEffector {
    /// Creates an unauthenticated HTTP effector targeting `<dsh_base_url>/api/runlark/dispatch`.
    ///
    /// # Panics
    /// Panics only if the TLS/HTTP client backend cannot be initialized.
    #[must_use]
    pub fn new(dsh_base_url: &str) -> Self {
        Self::with_token(dsh_base_url, None)
    }

    /// Creates an HTTP effector targeting `<dsh_base_url>/api/runlark/dispatch` with an optional
    /// DSH launch token used for `/connect` session cookie exchange and Bearer auth.
    ///
    /// # Panics
    /// Panics only if the TLS/HTTP client backend cannot be initialized.
    #[must_use]
    pub fn with_token(dsh_base_url: &str, token: Option<String>) -> Self {
        let origin = dsh_base_url.trim_end_matches('/').to_string();
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(Policy::none())
            .build()
            .expect("reqwest HTTP client must build");
        Self {
            client,
            endpoint: format!("{origin}/api/runlark/dispatch"),
            origin,
            token: token.filter(|t| !t.trim().is_empty()),
            cookie: RwLock::new(None),
        }
    }

    async fn cached_or_fresh_cookie(&self) -> Option<String> {
        let token = self.token.as_deref()?;
        if let Some(existing) = self.cookie.read().await.clone() {
            return Some(existing);
        }
        if let Ok(Some(fresh)) =
            authenticate_dsh_cookie(&self.client, &self.origin, Some(token)).await
        {
            *self.cookie.write().await = Some(fresh.clone());
            return Some(fresh);
        }
        None
    }

    async fn send_dispatch(
        &self,
        body: &HttpDispatchPayload<'_>,
    ) -> Result<reqwest::Response, reqwest::Error> {
        let cookie = self.cached_or_fresh_cookie().await;
        let mut req = self
            .client
            .post(&self.endpoint)
            .header(ORIGIN, &self.origin)
            .json(body);
        if let Some(ref c) = cookie {
            req = req.header(COOKIE, c);
        }
        if let Some(ref tok) = self.token {
            req = req.header(AUTHORIZATION, format!("Bearer {tok}"));
        }
        let resp = req.send().await?;

        if self.token.is_some()
            && matches!(
                resp.status(),
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
            )
        {
            *self.cookie.write().await = None;
            let refreshed = self.cached_or_fresh_cookie().await;
            let mut retry = self
                .client
                .post(&self.endpoint)
                .header(ORIGIN, &self.origin)
                .json(body);
            if let Some(ref c) = refreshed {
                retry = retry.header(COOKIE, c);
            }
            if let Some(ref tok) = self.token {
                retry = retry.header(AUTHORIZATION, format!("Bearer {tok}"));
            }
            return retry.send().await;
        }
        Ok(resp)
    }
}

impl OutboxEffector for HttpDshEffector {
    fn execute<'a>(
        &'a self,
        ctx: &'a DispatchContext,
    ) -> Pin<Box<dyn Future<Output = EffectOutcome> + Send + 'a>> {
        Box::pin(async move {
            let body = HttpDispatchPayload {
                command_id: ctx.command.command_id.as_ref(),
                session_id: ctx.command.session_id.as_ref(),
                run_id: ctx.command.run_id.as_ref(),
                command: &ctx.parsed,
                worktree_path: ctx
                    .worktree_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned()),
            };

            match self.send_dispatch(&body).await {
                Ok(resp) if resp.status().is_success() => EffectOutcome::Acked,
                Ok(resp)
                    if matches!(
                        resp.status(),
                        StatusCode::BAD_GATEWAY | StatusCode::GATEWAY_TIMEOUT
                    ) =>
                {
                    EffectOutcome::Unknown {
                        reason: format!("ambiguous DSH bridge status {}", resp.status()),
                    }
                }
                Ok(resp) => {
                    let status = resp.status();
                    let detail = resp.text().await.unwrap_or_default();
                    EffectOutcome::Failed {
                        error: format!("DSH bridge rejected command ({status}): {detail}"),
                    }
                }
                Err(err) if err.is_timeout() => EffectOutcome::Unknown {
                    reason: format!("DSH bridge request timed out: {err}"),
                },
                Err(err) if err.is_connect() => EffectOutcome::Failed {
                    error: format!("DSH bridge connection failed: {err}"),
                },
                Err(err) => EffectOutcome::Unknown {
                    reason: format!("ambiguous DSH bridge transport error: {err}"),
                },
            }
        })
    }
}

fn current_timestamp_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

enum LocalPrepPlan {
    EnsureWorktree {
        workspace: PathBuf,
        existing_worktree: String,
    },
    CleanupWorktrees {
        workspace: PathBuf,
        run_ids: Vec<runlark_domain::RunId>,
    },
    ExistingWorktree(Option<PathBuf>),
}

/// Durable command outbox dispatcher driving `pending -> dispatched -> acked | failed`
/// transitions and preserving `unknown` outcomes for reconciliation.
pub struct OutboxDispatcher {
    db_path: PathBuf,
    effector: Arc<dyn OutboxEffector>,
    config: OutboxConfig,
}

impl OutboxDispatcher {
    /// Creates a new outbox dispatcher backed by `db_path` and `effector`.
    #[must_use]
    pub fn new(db_path: PathBuf, effector: Arc<dyn OutboxEffector>, config: OutboxConfig) -> Self {
        Self {
            db_path,
            effector,
            config,
        }
    }

    /// Runs the background outbox loop until `shutdown` signals `true`.
    pub async fn run_loop(
        self: Arc<Self>,
        notify: Arc<Notify>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        if let Err(err) = self.dispatch_once().await {
            tracing::error!("Initial outbox dispatch failed: {err}");
        }

        loop {
            if *shutdown.borrow() {
                break;
            }

            tokio::select! {
                () = notify.notified() => {},
                () = tokio::time::sleep(self.config.poll_interval) => {},
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
            }

            if *shutdown.borrow() {
                break;
            }

            if let Err(err) = self.dispatch_once().await {
                tracing::error!("Outbox dispatch pass failed: {err}");
            }
        }
    }

    /// Scans `pending` and retry-eligible `failed` commands and dispatches them in FIFO order.
    ///
    /// # Errors
    /// Returns `StorageError` if opening `SQLite` or updating command rows fails.
    pub async fn dispatch_once(&self) -> Result<DispatchStats, StorageError> {
        let candidates = self.load_candidates()?;
        let mut stats = DispatchStats::default();

        for cmd in candidates {
            let now = current_timestamp_ms();
            if cmd.status == "failed"
                && now
                    < cmd
                        .updated_at
                        .0
                        .saturating_add(self.config.retry_backoff_ms)
            {
                continue;
            }

            let parsed: SessionCommand = match serde_json::from_str(&cmd.payload) {
                Ok(parsed) => parsed,
                Err(err) => {
                    self.record_failed(
                        &cmd.command_id,
                        &format!("invalid command payload: {err}"),
                    )?;
                    stats.attempted += 1;
                    stats.failed += 1;
                    continue;
                }
            };

            let (updated_cmd, plan) = self.begin_dispatch(&cmd, &parsed, now)?;
            stats.attempted += 1;

            let worktree_path = match self.execute_local_plan(&updated_cmd, plan).await {
                Ok(path) => path,
                Err(err) => {
                    self.record_failed(&updated_cmd.command_id, &err)?;
                    stats.failed += 1;
                    continue;
                }
            };

            let ctx = DispatchContext {
                command: updated_cmd.clone(),
                parsed,
                worktree_path,
            };

            match self.effector.execute(&ctx).await {
                EffectOutcome::Acked => {
                    self.record_acked(&updated_cmd.command_id)?;
                    stats.acked += 1;
                }
                EffectOutcome::Failed { error } => {
                    self.record_failed(&updated_cmd.command_id, &error)?;
                    stats.failed += 1;
                }
                EffectOutcome::Unknown { reason } => {
                    tracing::warn!(
                        command_id = %updated_cmd.command_id,
                        run_id = %updated_cmd.run_id,
                        %reason,
                        "Outbox dispatch outcome unknown; keeping dispatched state until reconciled"
                    );
                    stats.unknown += 1;
                }
            }
        }

        Ok(stats)
    }

    fn load_candidates(&self) -> Result<Vec<PersistedCommand>, StorageError> {
        let conn = open_sqlite(&self.db_path)?;
        CommandStore::list_dispatchable_commands(&conn)
    }

    fn begin_dispatch(
        &self,
        cmd: &PersistedCommand,
        parsed: &SessionCommand,
        now: u64,
    ) -> Result<(PersistedCommand, LocalPrepPlan), StorageError> {
        let conn = open_sqlite(&self.db_path)?;
        CommandStore::mark_dispatched(&conn, &cmd.command_id, Timestamp(now))?;
        let updated =
            CommandStore::get_command(&conn, &cmd.command_id)?.unwrap_or_else(|| cmd.clone());

        let plan = match parsed {
            SessionCommand::SendPrompt(_) | SessionCommand::ResumeSession(_) => {
                let session = SessionStore::get_session(&conn, &cmd.session_id)?;
                let run = SessionStore::get_run(&conn, &cmd.run_id)?;
                match (session, run) {
                    (Some(s), Some(r)) => LocalPrepPlan::EnsureWorktree {
                        workspace: PathBuf::from(s.cwd),
                        existing_worktree: r.worktree_path,
                    },
                    _ => LocalPrepPlan::ExistingWorktree(None),
                }
            }
            SessionCommand::ArchiveSession(_) if self.config.cleanup_worktree_on_archive => {
                match SessionStore::get_session(&conn, &cmd.session_id)? {
                    Some(session) => {
                        let runs = SessionStore::list_runs_for_session(&conn, &cmd.session_id)?;
                        LocalPrepPlan::CleanupWorktrees {
                            workspace: PathBuf::from(session.cwd),
                            run_ids: runs.into_iter().map(|r| r.id).collect(),
                        }
                    }
                    None => LocalPrepPlan::ExistingWorktree(None),
                }
            }
            _ => {
                let run = SessionStore::get_run(&conn, &cmd.run_id)?;
                LocalPrepPlan::ExistingWorktree(run.map(|r| PathBuf::from(r.worktree_path)))
            }
        };

        Ok((updated, plan))
    }

    async fn execute_local_plan(
        &self,
        cmd: &PersistedCommand,
        plan: LocalPrepPlan,
    ) -> Result<Option<PathBuf>, String> {
        match plan {
            LocalPrepPlan::EnsureWorktree {
                workspace,
                existing_worktree,
            } => {
                let run_id = cmd.run_id.clone();
                let resolution = tokio::task::spawn_blocking(move || {
                    WorktreeManager::ensure_worktree(&workspace, &run_id)
                })
                .await
                .map_err(|e| format!("worktree task panicked: {e}"))?
                .map_err(|e| e.to_string())?;

                let resolved_str = resolution.worktree_path.to_string_lossy().into_owned();
                if existing_worktree != resolved_str {
                    let conn = open_sqlite(&self.db_path).map_err(|e| e.to_string())?;
                    SessionStore::update_run_worktree(
                        &conn,
                        &cmd.run_id,
                        &resolved_str,
                        Timestamp(current_timestamp_ms()),
                    )
                    .map_err(|e| e.to_string())?;
                }
                Ok(Some(resolution.worktree_path))
            }
            LocalPrepPlan::CleanupWorktrees { workspace, run_ids } => {
                tokio::task::spawn_blocking(move || {
                    for run_id in run_ids {
                        let _ = WorktreeManager::remove_worktree(&workspace, &run_id, false)?;
                    }
                    Ok::<(), crate::worktree::WorktreeError>(())
                })
                .await
                .map_err(|e| format!("worktree cleanup task panicked: {e}"))?
                .map_err(|e| e.to_string())?;
                Ok(None)
            }
            LocalPrepPlan::ExistingWorktree(path) => Ok(path),
        }
    }

    fn record_acked(&self, command_id: &runlark_domain::CommandId) -> Result<(), StorageError> {
        let conn = open_sqlite(&self.db_path)?;
        CommandStore::mark_acked(&conn, command_id, Timestamp(current_timestamp_ms()))
    }

    fn record_failed(
        &self,
        command_id: &runlark_domain::CommandId,
        error: &str,
    ) -> Result<(), StorageError> {
        let conn = open_sqlite(&self.db_path)?;
        CommandStore::mark_failed(&conn, command_id, error, Timestamp(current_timestamp_ms()))
    }
}
