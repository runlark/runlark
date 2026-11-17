use reqwest::Client;
use runlark_cli::{AppState, VerificationService, build_router};
use runlark_domain::{
    Revision, Run, RunId, RunStatus, Session, SessionId, SessionStatus, TaskId, Timestamp,
};
use runlark_local::{SessionStore, initialize_schema, open_sqlite};
use serde_json::{Value, json};
use std::path::Path;
use tempfile::TempDir;

fn solo_default_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("workflows")
        .join("solo-default")
}

#[test]
fn plugin_cli_validate_and_inspect_verify_solo_default() {
    let pkg_dir = solo_default_dir();
    runlark_cli::plugin::run_plugin_inspect(&pkg_dir, true).expect("plugin inspect succeeds");
    runlark_cli::plugin::run_plugin_validate(&pkg_dir, true).expect("plugin validate succeeds");
}

#[tokio::test]
async fn verify_run_endpoint_executes_digest_pinned_wasm_validator() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("runlark.db");
    let cas_dir = dir.path().join("wasm-cas");
    let conn = open_sqlite(&db_path).expect("open db");
    initialize_schema(&conn).expect("init schema");

    let session = Session {
        id: SessionId::from("sess_verify_1"),
        task_id: TaskId::from("task_verify_1"),
        cwd: dir.path().display().to_string(),
        status: SessionStatus::Active,
        current_revision: Revision(1),
        active_run_id: Some(RunId::from("run_verify_1")),
        created_at: Timestamp(1_000),
        updated_at: Timestamp(1_000),
    };
    SessionStore::create_session(&conn, &session).expect("create session");

    let run = Run {
        id: RunId::from("run_verify_1"),
        session_id: SessionId::from("sess_verify_1"),
        status: RunStatus::Running,
        worktree_path: dir.path().display().to_string(),
        revision: Revision(1),
        created_at: Timestamp(1_000),
        updated_at: Timestamp(1_000),
        ..Run::default()
    };
    SessionStore::save_run(&conn, &run).expect("save run");
    drop(conn);

    let verification = VerificationService::with_cas_dir(&cas_dir).expect("init verification");
    verification
        .install_package_dir(&solo_default_dir())
        .expect("install solo-default");
    let state = AppState::with_verification(db_path, verification);

    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let client = Client::new();
    let base = format!("http://{addr}");

    let pass_resp = client
        .post(format!("{base}/api/v1/runs/run_verify_1/verify"))
        .json(&json!({
            "task_id": "task_verify_1",
            "changed_files": ["crates/runlark-wasm/src/lib.rs"],
            "checks": [{"name": "just check", "passed": true, "exit_code": 0}],
            "diff_bytes": 512,
            "repair_attempt": 0
        }))
        .send()
        .await
        .expect("verify pass request");
    assert_eq!(pass_resp.status(), 200);
    let pass_body: Value = pass_resp.json().await.expect("parse pass json");
    assert_eq!(pass_body["verdict"], "pass");
    assert_eq!(pass_body["package_name"], "solo-default");
    assert!(pass_body["fuel_consumed"].as_u64().unwrap_or(0) > 0);

    let fail_resp = client
        .post(format!("{base}/api/v1/runs/run_verify_1/verify"))
        .json(&json!({
            "task_id": "task_verify_1",
            "changed_files": [],
            "checks": [{"name": "clippy", "passed": false, "exit_code": 101}],
            "diff_bytes": 0,
            "repair_attempt": 1
        }))
        .send()
        .await
        .expect("verify fail request");
    assert_eq!(fail_resp.status(), 200);
    let fail_body: Value = fail_resp.json().await.expect("parse fail json");
    assert_eq!(fail_body["verdict"], "fail");
    assert!(
        !fail_body["findings"]
            .as_array()
            .expect("findings")
            .is_empty()
    );

    handle.abort();
}
