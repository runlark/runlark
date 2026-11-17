use reqwest::Client;
use runlark_cli::{
    AppState, LocalBridgeEffector, OutboxConfig, OutboxDispatcher, Scheduler, VerificationService,
    build_router,
};
use runlark_local::{initialize_schema, open_sqlite};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::watch;

fn solo_default_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("workflows")
        .join("solo-default")
}

fn init_git_repo(dir: &Path) {
    let run = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(out.status.success());
    };
    run(&["init"]);
    run(&["config", "user.email", "test@runlark.dev"]);
    run(&["config", "user.name", "Runlark Test"]);
    fs::write(dir.join("README.md"), "# Chat & CLI Test\n").expect("write");
    run(&["add", "README.md"]);
    run(&["commit", "-m", "init"]);
}

async fn setup_chat_harness(
    temp: &TempDir,
) -> (
    Client,
    String,
    watch::Sender<bool>,
    tokio::task::JoinHandle<()>,
) {
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("mkdir");
    init_git_repo(&repo_dir);

    let db_path = temp.path().join("cli.db");
    let cas_dir = temp.path().join("wasm-cas");
    let conn = open_sqlite(&db_path).expect("open");
    initialize_schema(&conn).expect("schema");
    drop(conn);

    let verification = VerificationService::with_cas_dir(&cas_dir).expect("init");
    verification
        .install_package_dir(&solo_default_dir())
        .expect("install");
    let state = AppState::with_verification(db_path.clone(), verification);

    let dispatcher = Arc::new(OutboxDispatcher::new(
        db_path,
        Arc::new(LocalBridgeEffector),
        OutboxConfig {
            poll_interval: Duration::from_millis(30),
            retry_backoff_ms: 0,
            cleanup_worktree_on_archive: false,
        },
    ));
    let scheduler = Arc::new(Scheduler::new(state.clone()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(dispatcher.run_loop(state.outbox_notify.clone(), shutdown_rx.clone()));
    tokio::spawn(scheduler.run_loop(state.scheduler_notify.clone(), shutdown_rx));

    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let base = format!("http://{addr}");
    let base_for_cmd = base.clone();
    let repo_str = repo_dir.to_string_lossy().into_owned();
    let proj_out = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_runlark"))
            .args([
                "--daemon-url",
                &base_for_cmd,
                "--json",
                "project",
                "create",
                "--id",
                "proj_cli",
                "--repo-root",
                &repo_str,
                "CLI Project",
            ])
            .output()
            .expect("run cli project create")
    })
    .await
    .expect("join");
    assert!(
        proj_out.status.success(),
        "cli failed: {}",
        String::from_utf8_lossy(&proj_out.stderr)
    );

    (Client::new(), base, shutdown_tx, server)
}

async fn post_chat(client: &Client, base: &str, input: &str, project_id: Option<&str>) -> Value {
    let mut body = json!({ "input": input });
    if let Some(pid) = project_id {
        body["project_id"] = json!(pid);
    }
    client
        .post(format!("{base}/api/v1/chat/dispatch"))
        .json(&body)
        .send()
        .await
        .expect("chat")
        .json()
        .await
        .expect("json")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unified_chat_dispatch_board_history_and_cli_commands_work_end_to_end() {
    let temp = TempDir::new().expect("temp dir");
    let (client, base, shutdown_tx, server) = setup_chat_harness(&temp).await;

    let fix_res = post_chat(&client, &base, "/fix Fix login redirect", Some("proj_cli")).await;
    assert_eq!(fix_res["command"], "fix");
    let task_id = fix_res["data"]["id"].as_str().expect("task_id").to_string();

    let plan_res = post_chat(
        &client,
        &base,
        &format!("/plan {task_id} Validate redirect URI"),
        None,
    )
    .await;
    assert_eq!(plan_res["command"], "plan");
    assert_eq!(plan_res["data"]["plan_version"], 2);

    let approve_res = post_chat(
        &client,
        &base,
        &format!("/plan approve {task_id} Looks solid"),
        None,
    )
    .await;
    assert_eq!(approve_res["command"], "plan_approve");
    assert!(
        approve_res["summary"]
            .as_str()
            .expect("summary")
            .contains("Looks solid")
    );
    let approved_plan_text = approve_res["data"]["plan_text"]
        .as_str()
        .expect("plan_text");
    assert!(approved_plan_text.contains("Validate redirect URI"));
    assert!(approved_plan_text.contains("Looks solid"));

    let run_res = post_chat(
        &client,
        &base,
        &format!("/run {task_id} --prompt \"Fix it\""),
        None,
    )
    .await;
    assert_eq!(run_res["command"], "run");
    let run_id = run_res["data"]["run"]["id"]
        .as_str()
        .expect("run_id")
        .to_string();
    let sess_id = run_res["data"]["run"]["session_id"]
        .as_str()
        .expect("session_id");
    let wt_path = PathBuf::from(
        run_res["data"]["run"]["worktree_path"]
            .as_str()
            .expect("worktree_path"),
    );
    fs::write(wt_path.join("README.md"), "# Redirect fixed\n").expect("edit worktree");

    // Turn ends on task-backed run -> triggers verification + advances to Acceptance!
    let turn_end_res: Value = client
        .post(format!("{base}/api/v1/dsh/events"))
        .json(&json!({ "TurnEnded": { "session_id": sess_id, "run_id": run_id, "outcome": "Completed" } }))
        .send()
        .await
        .expect("turn end")
        .json()
        .await
        .expect("json");
    assert_eq!(turn_end_res["run_status"], "waiting_for_approval");

    let rej_res = post_chat(
        &client,
        &base,
        &format!("/reject {task_id} Add unit test"),
        None,
    )
    .await;
    assert_eq!(rej_res["command"], "reject");
    assert_eq!(rej_res["data"]["task"]["stage"], "in_progress");

    client
        .post(format!("{base}/api/v1/dsh/events"))
        .json(&json!({ "TurnEnded": { "session_id": sess_id, "run_id": run_id, "outcome": "Completed" } }))
        .send()
        .await
        .expect("turn end 2");

    let acc_res = post_chat(
        &client,
        &base,
        &format!("/accept {task_id} Looks good"),
        None,
    )
    .await;
    assert_eq!(acc_res["command"], "accept");
    assert_eq!(acc_res["data"]["task"]["stage"], "done");

    assert_cli_and_history(&client, &base, &run_id).await;

    let _ = shutdown_tx.send(true);
    server.abort();
}

async fn assert_cli_and_history(client: &Client, base: &str, run_id: &str) {
    let hist: Value = client
        .get(format!("{base}/api/v1/runs/{run_id}/history"))
        .send()
        .await
        .expect("history")
        .json()
        .await
        .expect("json");
    let digest = hist["artifacts"][0]["digest_sha256"]
        .as_str()
        .expect("digest");
    let art_resp = client
        .get(format!("{base}/api/v1/artifacts/{digest}"))
        .send()
        .await
        .expect("get artifact");
    assert_eq!(art_resp.status(), 200);

    let base_board = base.to_string();
    let board_out = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_runlark"))
            .args(["--daemon-url", &base_board, "--json", "task", "board"])
            .output()
            .expect("cli board")
    })
    .await
    .expect("join");
    assert!(board_out.status.success());
    let board_json: Value = serde_json::from_slice(&board_out.stdout).expect("board json");
    assert_eq!(
        board_json["columns"]["done"]
            .as_array()
            .expect("done")
            .len(),
        1
    );

    let base_plugins = base.to_string();
    let plugins_out = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_runlark"))
            .args(["--daemon-url", &base_plugins, "--json", "plugin", "list"])
            .output()
            .expect("cli plugin list")
    })
    .await
    .expect("join");
    assert!(plugins_out.status.success());
}
