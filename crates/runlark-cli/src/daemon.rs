use crate::dsh_subscriber::{DshEndpoint, DshSubscriber};
use crate::outbox::{
    HttpDshEffector, LocalBridgeEffector, OutboxConfig, OutboxDispatcher, OutboxEffector,
};
use crate::routes::{commands, dsh_events, events, health, questions, runs, sessions};
use crate::state::AppState;
use axum::Router;
use axum::routing::{get, post};
use runlark_domain::Timestamp;
use runlark_local::{ReconciliationEngine, initialize_schema, open_sqlite};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::signal;
use tokio::sync::watch;
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
        .route(
            "/api/v1/sessions/{id}/questions",
            get(questions::list_session_questions),
        )
        .route(
            "/api/v1/sessions/{id}/events",
            get(events::get_session_events),
        )
        .route("/api/v1/commands", post(commands::submit_command))
        .route("/api/v1/runs/{id}", get(runs::get_run))
        .route("/api/v1/runs/{id}/events", get(events::get_events))
        .route("/api/v1/questions", get(questions::list_questions))
        .route(
            "/api/v1/questions/{id}/answer",
            post(questions::answer_question),
        )
        .route("/api/v1/dsh/events", post(dsh_events::ingest_dsh_event))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Runs the Runlark daemon server on the specified host and port, reading optional
/// `RUNLARK_DSH_URL` and `RUNLARK_DSH_TOKEN` environment variables.
///
/// # Errors
/// Returns an error if database initialization, network binding, or server execution fails.
pub async fn run_daemon(
    host: String,
    port: u16,
    data_dir: PathBuf,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let dsh_url = std::env::var("RUNLARK_DSH_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let dsh_token = std::env::var("RUNLARK_DSH_TOKEN")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    run_daemon_with_dsh(host, port, data_dir, dsh_url, dsh_token).await
}

/// Runs the Runlark daemon server with an optional explicit DSH URL and launch token.
///
/// # Errors
/// Returns an error if database initialization, network binding, or server execution fails.
pub async fn run_daemon_with_dsh(
    host: String,
    port: u16,
    data_dir: PathBuf,
    dsh_url: Option<String>,
    dsh_token: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let dsh_url = dsh_url.or_else(|| std::env::var("RUNLARK_DSH_URL").ok());
    let dsh_token = dsh_token.or_else(|| std::env::var("RUNLARK_DSH_TOKEN").ok());
    std::fs::create_dir_all(&data_dir)?;
    let db_path = data_dir.join("runlark.db");

    tracing::info!("Initializing SQLite database at {:?}", db_path);
    let mut conn = open_sqlite(&db_path)?;
    initialize_schema(&conn)?;

    let now = AppState::current_timestamp_ms();
    let inflight = ReconciliationEngine::reconcile_on_startup(&mut conn, Timestamp(now))?;
    if !inflight.active_runs.is_empty() {
        tracing::warn!(
            "Reconciled {} active runs to interrupted on startup",
            inflight.active_runs.len()
        );
    }
    if !inflight.waiting_runs.is_empty() {
        tracing::info!(
            "Preserved {} runs waiting for human approval ({} pending questions)",
            inflight.waiting_runs.len(),
            inflight.pending_questions.len()
        );
    }

    let state = AppState::new(db_path.clone());
    let resolved_dsh = dsh_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|url| DshEndpoint::parse(url, dsh_token.clone()));

    let effector: Arc<dyn OutboxEffector> = match &resolved_dsh {
        Some(endpoint) => {
            tracing::info!("Outbox configured with DSH bridge URL: {}", endpoint.origin);
            Arc::new(HttpDshEffector::with_token(
                &endpoint.origin,
                endpoint.token.clone(),
            ))
        }
        None => Arc::new(LocalBridgeEffector),
    };

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let dispatcher = Arc::new(OutboxDispatcher::new(
        db_path,
        effector,
        OutboxConfig::default(),
    ));
    let outbox_task =
        tokio::spawn(dispatcher.run_loop(state.outbox_notify.clone(), shutdown_rx.clone()));

    let dsh_task = dsh_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|url| {
            tracing::info!("Subscribing to DSH session/follow stream at {}", url);
            let subscriber = DshSubscriber::new(state.clone(), url, dsh_token);
            tokio::spawn(subscriber.run_loop(state.dsh_notify.clone(), shutdown_rx))
        });

    let app = build_router(state.clone());

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Runlark daemon listening on http://{}", addr);

    let shutdown_state = state.clone();
    let serve_result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            shutdown_state.signal_shutdown();
        })
        .await;

    let _ = shutdown_tx.send(true);
    state.signal_shutdown();
    if let Err(err) = outbox_task.await {
        tracing::warn!("Outbox task terminated with join error: {err}");
    }
    if let Some(handle) = dsh_task
        && let Err(err) = handle.await
    {
        tracing::warn!("DSH subscriber task terminated with join error: {err}");
    }

    serve_result?;
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
