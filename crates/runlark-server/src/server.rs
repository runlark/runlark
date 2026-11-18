use crate::cas::ServerCasStore;
use crate::config::ServerConfig;
use crate::error::ServerError;
use crate::pg::{create_pool, run_migrations, runners};
use crate::routes;
use axum::Router;
use axum::routing::{get, patch, post, put};
use deadpool_postgres::Pool;
use runlark_domain::Timestamp;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

/// Shared application state for `runlark-server` HTTP handlers.
#[derive(Clone)]
pub struct ServerState {
    pub pool: Pool,
    pub cas: ServerCasStore,
    pub config: Arc<ServerConfig>,
}

impl ServerState {
    /// Builds a [`ServerState`] from `config` and runs `PostgreSQL` schema migrations.
    ///
    /// # Errors
    /// Returns [`ServerError`] if `PostgreSQL` pool creation or migrations fail.
    pub async fn connect_and_migrate(config: ServerConfig) -> Result<Self, ServerError> {
        let pool = create_pool(&config.postgres_url)?;
        run_migrations(&pool).await?;
        let cas = ServerCasStore::new(&config.cas_root);
        Ok(Self {
            pool,
            cas,
            config: Arc::new(config),
        })
    }
}

/// Builds the Axum router implementing the unified SaaS/self-host collaboration protocol.
pub fn build_router(state: ServerState) -> Router {
    Router::new()
        .route("/health", get(routes::health::health_check))
        .route(
            "/api/v1/auth/whoami",
            get(routes::workspaces::whoami).post(routes::workspaces::whoami),
        )
        .route(
            "/api/v1/workspaces",
            post(routes::workspaces::create_workspace),
        )
        .route(
            "/api/v1/workspaces/{wid}",
            get(routes::workspaces::get_workspace),
        )
        .route(
            "/api/v1/workspaces/{wid}/members",
            get(routes::workspaces::list_members),
        )
        .route(
            "/api/v1/workspaces/{wid}/members/{uid}",
            patch(routes::workspaces::update_member_role),
        )
        .route(
            "/api/v1/workspaces/{wid}/invitations",
            post(routes::workspaces::create_invitation),
        )
        .route(
            "/api/v1/workspaces/{wid}/invitations/{invite_id}/revoke",
            post(routes::workspaces::revoke_invitation),
        )
        .route(
            "/api/v1/invitations/accept",
            post(routes::workspaces::accept_invitation),
        )
        .route(
            "/api/v1/invitations/{token}/accept",
            post(routes::workspaces::accept_invitation_by_path),
        )
        .route(
            "/api/v1/workspaces/{wid}/projects/promote",
            post(routes::workspaces::promote_project),
        )
        .route(
            "/api/v1/workspaces/{wid}/runners",
            get(routes::runners::list_runners),
        )
        .route(
            "/api/v1/workspaces/{wid}/runners/register",
            post(routes::runners::register_runner),
        )
        .route(
            "/api/v1/workspaces/{wid}/runners/{rid}/heartbeat",
            post(routes::runners::heartbeat_runner),
        )
        .route(
            "/api/v1/workspaces/{wid}/runners/sweep",
            post(routes::runners::sweep_runners),
        )
        .route(
            "/api/v1/workspaces/{wid}/runs/{run_id}/assign",
            post(routes::runners::assign_runner),
        )
        .route(
            "/api/v1/workspaces/{wid}/runs/{run_id}/runner-state",
            post(routes::runners::report_runner_state),
        )
        .route(
            "/api/v1/workspaces/{wid}/commands",
            post(routes::commands::submit_command),
        )
        .route(
            "/api/v1/workspaces/{wid}/tasks/{task_id}/comments",
            get(routes::commands::list_comments),
        )
        .route(
            "/api/v1/workspaces/{wid}/sync/snapshot",
            get(routes::sync::get_snapshot),
        )
        .route(
            "/api/v1/workspaces/{wid}/sync/events",
            get(routes::sync::get_workspace_events),
        )
        .route(
            "/api/v1/workspaces/{wid}/runs/{run_id}/events",
            get(routes::sync::get_run_events),
        )
        .route(
            "/api/v1/workspaces/{wid}/artifacts/{digest}",
            put(routes::cas::upload_artifact).get(routes::cas::download_artifact),
        )
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Runs the `runlark-server` HTTP daemon until SIGINT/SIGTERM.
///
/// # Errors
/// Returns an error if database setup, TCP binding, or HTTP serving fails.
pub async fn run_server(
    config: ServerConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    std::fs::create_dir_all(&config.cas_root)?;
    let addr = format!("{}:{}", config.host, config.port);
    let state = ServerState::connect_and_migrate(config).await?;
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Runlark collaboration server listening on http://{addr}");

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let sweeper = spawn_runner_sweeper(state.clone(), shutdown_rx);

    axum::serve(listener, build_router(state))
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            let _ = shutdown_tx.send(true);
        })
        .await?;
    let _ = sweeper.await;
    Ok(())
}

fn spawn_runner_sweeper(
    state: ServerState,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let timeout_ms = state.config.runner_heartbeat_timeout_ms;
    let interval_ms = (timeout_ms / 2).clamp(250, 10_000);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms));
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let now_ms = ServerConfig::current_timestamp_ms();
                    let stale_before = Timestamp(now_ms.saturating_sub(timeout_ms));
                    if let Err(err) = runners::sweep_all_stale_runners(
                        &state.pool,
                        stale_before,
                        Timestamp(now_ms),
                    )
                    .await
                    {
                        tracing::warn!("background runner heartbeat sweep failed: {err}");
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
            }
        }
    })
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
