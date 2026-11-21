use crate::dsh_subscriber::{DshEndpoint, DshSubscriber};
use crate::outbox::{
    HttpDshEffector, LocalBridgeEffector, OutboxConfig, OutboxDispatcher, OutboxEffector,
};
use crate::routes::{
    board, chat, commands, dsh_events, events, health, plugins, projects, questions, run_history,
    runs, sessions, task_actions, tasks, team, team_comments, team_drafts, team_runners,
    team_workspaces, verification,
};
use crate::runner_agent::RunnerAgentWorker;
use crate::scheduler::Scheduler;
use crate::state::AppState;
use crate::team_sync::TeamSyncWorker;
use crate::verification::VerificationService;
use axum::Router;
use axum::routing::{delete, get, patch, post};
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
            "/api/v1/projects",
            get(projects::list_projects).post(projects::create_project),
        )
        .route("/api/v1/projects/{id}", get(projects::get_project))
        .route("/api/v1/board", get(board::get_board))
        .route("/api/v1/scheduler/status", get(board::get_scheduler_status))
        .route(
            "/api/v1/tasks",
            get(tasks::list_tasks).post(tasks::create_task),
        )
        .route("/api/v1/tasks/{id}", get(tasks::get_task))
        .route("/api/v1/tasks/{id}/move", post(tasks::move_task))
        .route("/api/v1/tasks/{id}/clarify", post(tasks::clarify_task))
        .route("/api/v1/tasks/{id}/plan", post(tasks::plan_task))
        .route("/api/v1/tasks/{id}/start", post(task_actions::start_task))
        .route("/api/v1/tasks/{id}/accept", post(task_actions::accept_task))
        .route("/api/v1/tasks/{id}/reject", post(task_actions::reject_task))
        .route("/api/v1/tasks/{id}/pr", post(task_actions::create_task_pr))
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
        .route("/api/v1/runs", get(run_history::list_runs))
        .route("/api/v1/runs/{id}", get(runs::get_run))
        .route("/api/v1/runs/{id}/events", get(events::get_events))
        .route(
            "/api/v1/runs/{id}/history",
            get(run_history::get_run_history),
        )
        .route("/api/v1/runs/{id}/diff", get(run_history::get_run_diff))
        .route(
            "/api/v1/runs/{id}/attempts",
            get(run_history::list_run_attempts).post(run_history::record_run_attempt),
        )
        .route("/api/v1/runs/{id}/verify", post(verification::verify_run))
        .route("/api/v1/artifacts/{digest}", get(run_history::get_artifact))
        .route("/api/v1/questions", get(questions::list_questions))
        .route(
            "/api/v1/questions/{id}/answer",
            post(questions::answer_question),
        )
        .route("/api/v1/chat/dispatch", post(chat::chat_dispatch))
        .route(
            "/api/v1/plugins",
            get(plugins::list_plugins).post(plugins::install_plugin),
        )
        .route("/api/v1/plugins/validate", post(plugins::validate_plugin))
        .route("/api/v1/dsh/events", post(dsh_events::ingest_dsh_event))
        .merge(team_routes())
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

fn team_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/team/login", post(team::team_login))
        .route("/api/v1/team/status", get(team::team_status))
        .route("/api/v1/team/sync", post(team::trigger_team_sync))
        .route(
            "/api/v1/team/workspaces",
            post(team_workspaces::create_workspace),
        )
        .route(
            "/api/v1/team/workspaces/{wid}",
            get(team_workspaces::get_workspace),
        )
        .route(
            "/api/v1/team/workspaces/{wid}/members",
            get(team_workspaces::list_members),
        )
        .route(
            "/api/v1/team/workspaces/{wid}/members/{uid}",
            patch(team_workspaces::update_member_role),
        )
        .route(
            "/api/v1/team/workspaces/{wid}/invitations",
            get(team_workspaces::list_invitations).post(team_workspaces::create_invitation),
        )
        .route(
            "/api/v1/team/workspaces/{wid}/invitations/{invite_id}/revoke",
            post(team_workspaces::revoke_invitation),
        )
        .route(
            "/api/v1/team/invitations/accept",
            post(team_workspaces::accept_invitation),
        )
        .route(
            "/api/v1/projects/{id}/promote-team",
            post(team::promote_project_to_team),
        )
        .route(
            "/api/v1/team/drafts",
            get(team_drafts::list_drafts).post(team_drafts::create_draft),
        )
        .route(
            "/api/v1/team/drafts/{id}/retry",
            post(team_drafts::retry_draft),
        )
        .route(
            "/api/v1/team/drafts/{id}",
            delete(team_drafts::delete_draft),
        )
        .route(
            "/api/v1/tasks/{id}/comments",
            get(team_comments::list_task_comments).post(team_comments::add_task_comment),
        )
        .route("/api/v1/team/runners", get(team_runners::list_runners))
        .route(
            "/api/v1/team/runners/register",
            post(team_runners::register_runner),
        )
        .route(
            "/api/v1/team/runners/status",
            get(team_runners::runner_status),
        )
        .route(
            "/api/v1/team/runs/{run_id}/assign",
            post(team_runners::assign_runner),
        )
        .route(
            "/api/v1/team/runs/{run_id}/cancel-unreachable",
            post(team_runners::cancel_unreachable_run),
        )
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700));
    }
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

    let verification_service = init_verification_service(&data_dir)?;
    let state = AppState::with_verification(db_path.clone(), verification_service);
    let resolved_dsh = dsh_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|url| DshEndpoint::parse(url, dsh_token.clone()));
    let effector: Arc<dyn OutboxEffector> = match &resolved_dsh {
        Some(endpoint) => {
            tracing::info!(
                "Outbox configured with DSH bridge URL: {}",
                endpoint.log_origin()
            );
            Arc::new(HttpDshEffector::with_token(
                &endpoint.origin,
                endpoint.token.clone(),
            ))
        }
        None => Arc::new(LocalBridgeEffector),
    };

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Runlark daemon listening on http://{}", addr);

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let workers = spawn_daemon_workers(&state, db_path, effector, resolved_dsh, shutdown_rx);

    let app = build_router(state.clone());
    let shutdown_state = state.clone();
    let serve_result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            shutdown_state.signal_shutdown();
        })
        .await;

    let _ = shutdown_tx.send(true);
    state.signal_shutdown();
    join_daemon_workers(workers).await;

    serve_result?;
    tracing::info!("Runlark daemon gracefully stopped.");
    Ok(())
}

struct DaemonWorkers {
    outbox: tokio::task::JoinHandle<()>,
    scheduler: tokio::task::JoinHandle<()>,
    team_sync: tokio::task::JoinHandle<()>,
    runner: tokio::task::JoinHandle<()>,
    dsh: Option<tokio::task::JoinHandle<()>>,
}

fn init_verification_service(
    data_dir: &std::path::Path,
) -> Result<VerificationService, Box<dyn std::error::Error + Send + Sync>> {
    let verification_service = VerificationService::with_cas_dir(data_dir.join("wasm-cas"))?;
    let default_workflow = std::env::var("RUNLARK_WORKFLOW_PACKAGE")
        .ok()
        .map_or_else(|| PathBuf::from("workflows/solo-default"), PathBuf::from);
    if default_workflow.join("manifest.json").exists() {
        let pin = verification_service.install_package_dir(&default_workflow)?;
        tracing::info!(
            "Installed workflow package {}@{} (digest: {})",
            pin.name,
            pin.version,
            pin.package_digest
        );
    }
    Ok(verification_service)
}

fn spawn_daemon_workers(
    state: &AppState,
    db_path: PathBuf,
    effector: Arc<dyn OutboxEffector>,
    resolved_dsh: Option<DshEndpoint>,
    shutdown_rx: watch::Receiver<bool>,
) -> DaemonWorkers {
    let dispatcher = Arc::new(OutboxDispatcher::new(
        db_path,
        effector,
        OutboxConfig::default(),
    ));
    let outbox =
        tokio::spawn(dispatcher.run_loop(state.outbox_notify.clone(), shutdown_rx.clone()));
    let scheduler = tokio::spawn(
        Arc::new(Scheduler::new(state.clone()))
            .run_loop(state.scheduler_notify.clone(), shutdown_rx.clone()),
    );
    let team_sync = tokio::spawn(
        Arc::new(TeamSyncWorker::new(state.clone()))
            .run_loop(state.team_sync_notify.clone(), shutdown_rx.clone()),
    );
    let runner = tokio::spawn(
        Arc::new(RunnerAgentWorker::new(state.clone()))
            .run_loop(state.runner_notify.clone(), shutdown_rx.clone()),
    );
    let dsh = resolved_dsh.map(|endpoint| {
        tracing::info!(
            "Subscribing to DSH session/follow stream at {}",
            endpoint.log_origin()
        );
        let subscriber = DshSubscriber::from_endpoint(state.clone(), endpoint);
        tokio::spawn(subscriber.run_loop(state.dsh_notify.clone(), shutdown_rx))
    });
    DaemonWorkers {
        outbox,
        scheduler,
        team_sync,
        runner,
        dsh,
    }
}

async fn join_daemon_workers(workers: DaemonWorkers) {
    for (name, handle) in [
        ("Outbox", workers.outbox),
        ("Scheduler", workers.scheduler),
        ("Team sync", workers.team_sync),
        ("Runner agent", workers.runner),
    ] {
        if let Err(err) = handle.await {
            tracing::warn!("{name} task terminated with join error: {err}");
        }
    }
    if let Some(handle) = workers.dsh
        && let Err(err) = handle.await
    {
        tracing::warn!("DSH subscriber task terminated with join error: {err}");
    }
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
