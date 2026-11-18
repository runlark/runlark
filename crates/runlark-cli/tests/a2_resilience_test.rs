mod a2_team_mock;

use a2_team_mock::{
    MockTeamServerState, mock_commands, mock_events, mock_snapshot, mock_whoami, spawn_daemon,
};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use reqwest::Client;
use runlark_cli::RunnerAgentWorker;
use runlark_domain::{
    EventSeq, LeaseEpoch, RunId, RunnerEventInput, RunnerId, RunnerRegistration, RunnerRunReport,
    RunnerStatus, Timestamp, UserId, WorkspaceId,
};
use runlark_local::{RunnerBufferStore, SyncStore, TeamAuthSession, TeamAuthStore, open_sqlite};
use serde_json::json;
use tempfile::TempDir;

async fn spawn_err_and_ok_servers() -> (
    String,
    tokio::task::JoinHandle<()>,
    String,
    runlark_domain::WorkspaceSnapshot,
    tokio::task::JoinHandle<()>,
) {
    let err_app = Router::new()
        .route("/api/v1/auth/whoami", get(mock_whoami))
        .route(
            "/api/v1/workspaces/{wid}/sync/snapshot",
            get(|| async {
                (
                    StatusCode::FORBIDDEN,
                    Json(json!({ "error": "forbidden", "message": "not a member" })),
                )
            }),
        )
        .route(
            "/api/v1/workspaces/{wid}/runners/{rid}/heartbeat",
            post(|| async {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "error": "unauthorized", "message": "expired" })),
                )
            }),
        )
        .route(
            "/api/v1/workspaces/{wid}/runs/{rid}/runner-state",
            post(|| async {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "error": "unauthorized", "message": "expired" })),
                )
            }),
        )
        .with_state(MockTeamServerState::new("ws_bad", "Bad", "bad", None));
    let err_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let err_url = format!("http://{}", err_listener.local_addr().unwrap());
    let err_handle = tokio::spawn(async move {
        let _ = axum::serve(err_listener, err_app).await;
    });

    let ok_state = MockTeamServerState::new("ws_ok", "OK", "ok", None);
    let ok_snap = ok_state.inner.lock().await.snapshot.clone();
    let ok_app = Router::new()
        .route("/api/v1/workspaces/{wid}/sync/snapshot", get(mock_snapshot))
        .route("/api/v1/workspaces/{wid}/sync/events", get(mock_events))
        .route("/api/v1/workspaces/{wid}/commands", post(mock_commands))
        .route(
            "/api/v1/workspaces/{wid}/runners/{rid}/heartbeat",
            post(|| async {
                Json(RunnerRegistration {
                    id: RunnerId::from("rnr_ok"),
                    workspace_id: WorkspaceId::from("ws_ok"),
                    owner_user_id: UserId::from("usr_owner"),
                    name: "OK".to_string(),
                    status: RunnerStatus::Online,
                    last_heartbeat_at: Timestamp(1_000),
                    created_at: Timestamp(1_000),
                })
            }),
        )
        .with_state(ok_state);
    let ok_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ok_url = format!("http://{}", ok_listener.local_addr().unwrap());
    let ok_handle = tokio::spawn(async move {
        let _ = axum::serve(ok_listener, ok_app).await;
    });

    (err_url, err_handle, ok_url, ok_snap, ok_handle)
}

#[tokio::test]
async fn team_login_rolls_back_on_sync_failure_and_runner_tick_isolates_session_errors() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("runlark.db");
    let (daemon_url, state, _sd, daemon_handle) = spawn_daemon(&db_path).await;
    let (err_url, err_handle, ok_url, ok_snap, ok_handle) = spawn_err_and_ok_servers().await;

    let login_fail = Client::new()
        .post(format!("{daemon_url}/api/v1/team/login"))
        .json(&json!({ "server_url": err_url, "bearer_token": "tok", "workspace_id": "ws_bad" }))
        .send()
        .await
        .unwrap();
    assert_eq!(login_fail.status(), 403);
    assert!(
        TeamAuthStore::get_by_server(&open_sqlite(&db_path).unwrap(), &err_url)
            .unwrap()
            .is_none()
    );

    let mut conn = open_sqlite(&db_path).unwrap();
    SyncStore::apply_snapshot_atomic(&mut conn, &ok_url, &ok_snap, Timestamp(500)).unwrap();
    for (url, wid, rid, ts) in [
        (&err_url, "ws_bad", "rnr_bad", 2_000),
        (&ok_url, "ws_ok", "rnr_ok", 1_000),
    ] {
        TeamAuthStore::save_session(
            &conn,
            &TeamAuthSession {
                server_url: url.clone(),
                user_id: UserId::from("usr_owner"),
                display_name: "Owner".to_string(),
                bearer_token: "tok".to_string(),
                default_workspace_id: Some(WorkspaceId::from(wid)),
                runner_id: Some(RunnerId::from(rid)),
                runner_name: Some("R".to_string()),
                updated_at: Timestamp(ts),
            },
        )
        .unwrap();
    }
    RunnerBufferStore::buffer_report(
        &conn,
        &WorkspaceId::from("ws_bad"),
        &RunId::from("run_bad"),
        &RunnerRunReport {
            runner_id: RunnerId::from("rnr_bad"),
            lease_epoch: LeaseEpoch(1),
            status: None,
            workflow_stage: None,
            head_sha: None,
            base_sha: None,
            events: vec![RunnerEventInput {
                seq: EventSeq(1),
                event_type: "step".to_string(),
                payload_json: "{}".to_string(),
                created_at: Timestamp(2_000),
            }],
        },
    )
    .unwrap();
    drop(conn);

    let summary = RunnerAgentWorker::new(state).tick_once().await.unwrap();
    assert_eq!(summary.heartbeats_sent, 1);

    daemon_handle.abort();
    err_handle.abort();
    ok_handle.abort();
}

#[tokio::test]
async fn cli_accept_invite_reports_token_saved_status() {
    let app = Router::new().route(
        "/api/v1/team/invitations/accept",
        post(|| async {
            Json(json!({
                "member": {
                    "workspace_id": "ws_inv",
                    "user_id": "usr_bob",
                    "display_name": "Bob",
                    "role": "member",
                    "joined_at": 1000
                },
                "server_url": "http://127.0.0.1:9999",
                "token_saved": false
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let srv = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let out = tokio::task::spawn_blocking(move || {
        std::process::Command::new(env!("CARGO_BIN_EXE_runlark"))
            .env("RUNLARK_INVITE_TOKEN", "inv_tok_env")
            .args([
                "--daemon-url",
                &base,
                "team",
                "accept-invite",
                "--server",
                "http://127.0.0.1:9999",
                "--name",
                "Bob",
            ])
            .output()
            .expect("run cli")
    })
    .await
    .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("session token NOT saved"), "got: {stdout}");
    srv.abort();
}
