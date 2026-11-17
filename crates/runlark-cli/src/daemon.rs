use crate::routes::{commands, events, health, sessions};
use crate::state::AppState;
use axum::Router;
use axum::routing::{get, post};
use runlark_domain::Timestamp;
use runlark_local::{ReconciliationEngine, initialize_schema, open_sqlite};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::signal;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health_check))
        .route(
            "/api/v1/sessions",
            post(sessions::create_session).get(sessions::list_sessions),
        )
        .route("/api/v1/sessions/{id}", get(sessions::get_session))
        .route("/api/v1/commands", post(commands::submit_command))
        .route("/api/v1/runs/{id}/events", get(events::get_events))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

fn current_timestamp() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

/// Runs the Runlark daemon server on the specified host and port.
///
/// # Errors
/// Returns an error if database initialization, network binding, or server execution fails.
pub async fn run_daemon(
    host: String,
    port: u16,
    data_dir: PathBuf,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    std::fs::create_dir_all(&data_dir)?;
    let db_path = data_dir.join("runlark.db");

    tracing::info!("Initializing SQLite database at {:?}", db_path);
    let mut conn = open_sqlite(&db_path)?;
    initialize_schema(&conn)?;

    let now = current_timestamp();
    let inflight = ReconciliationEngine::reconcile_on_startup(&mut conn, Timestamp(now))?;
    if !inflight.active_runs.is_empty() {
        tracing::warn!(
            "Reconciled {} active runs to interrupted on startup",
            inflight.active_runs.len()
        );
    }

    let state = AppState::new(db_path);
    let app = build_router(state);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Runlark daemon listening on http://{}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("Runlark daemon gracefully stopped.");
    Ok(())
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
        () = ctrl_c => {
            tracing::info!("Received Ctrl+C signal, shutting down...");
        },
        () = terminate => {
            tracing::info!("Received SIGTERM signal, shutting down...");
        },
    }
}
