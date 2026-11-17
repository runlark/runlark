use reqwest::Client;
use runlark_cli::{AppState, Scheduler, VerificationService, build_router};
use runlark_domain::{RunStatus, Timestamp};
use runlark_local::{RunQueries, SessionStore, initialize_schema, open_sqlite};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;

fn init_git_repo(dir: &Path) {
    let run = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(out.status.success(), "git {args:?} failed");
    };
    run(&["init"]);
    run(&["config", "user.email", "test@runlark.dev"]);
    run(&["config", "user.name", "Runlark Test"]);
    fs::write(dir.join("README.md"), "# Regression Test\n").expect("write readme");
    run(&["add", "README.md"]);
    run(&["commit", "-m", "init"]);
}

async fn start_server(
    temp: &TempDir,
) -> (
    Client,
    String,
    AppState,
    std::path::PathBuf,
    tokio::task::JoinHandle<()>,
) {
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("mkdir");
    init_git_repo(&repo_dir);

    let db_path = temp.path().join("reg.db");
    let conn = open_sqlite(&db_path).expect("open db");
    initialize_schema(&conn).expect("schema");
    drop(conn);

    let pkg_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("workflows")
        .join("solo-default");
    let verification =
        VerificationService::with_cas_dir(temp.path().join("wasm-cas")).expect("cas dir");
    verification
        .install_package_dir(&pkg_dir)
        .expect("install solo-default");

    let state = AppState::with_verification(db_path, verification);
    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (
        Client::new(),
        format!("http://{addr}"),
        state,
        repo_dir,
        handle,
    )
}

async fn post_json(client: &Client, url: String, body: &Value) -> (reqwest::StatusCode, Value) {
    let resp = client.post(&url).json(body).send().await.expect("post");
    let status = resp.status();
    let text = resp.text().await.expect("text");
    let json = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("non-json response from {url} ({status}): {text} ({e})"));
    (status, json)
}

async fn assert_idempotent_post(
    client: &Client,
    url: &str,
    body: &Value,
    expected_first_status: u16,
    expected_replay_status: u16,
    conflict_body: Option<&Value>,
) -> Value {
    let (status, first) = post_json(client, url.to_string(), body).await;
    assert_eq!(status.as_u16(), expected_first_status);
    tokio::time::sleep(Duration::from_millis(12)).await;
    let (replay_status, _) = post_json(client, url.to_string(), body).await;
    assert_eq!(replay_status.as_u16(), expected_replay_status);
    if let Some(bad) = conflict_body {
        let (conflict_status, _) = post_json(client, url.to_string(), bad).await;
        assert_eq!(conflict_status, 409);
    }
    first
}

async fn assert_project_idempotency(client: &Client, base: &str, repo_dir: &Path) -> String {
    let proj_body = json!({
        "command_id": "cmd_retry_proj",
        "name": "RegRepo",
        "repo_root": repo_dir.display().to_string(),
        "default_branch": "main"
    });
    let conflict = json!({
        "command_id": "cmd_retry_proj",
        "name": "ChangedName",
        "repo_root": repo_dir.display().to_string(),
        "default_branch": "main"
    });
    let first = assert_idempotent_post(
        client,
        &format!("{base}/api/v1/projects"),
        &proj_body,
        201,
        200,
        Some(&conflict),
    )
    .await;
    first["project_id"]
        .as_str()
        .expect("project_id")
        .to_string()
}

async fn assert_task_plan_and_live_verify(
    client: &Client,
    base: &str,
    proj_id: &str,
) -> (String, String) {
    let task_body = json!({
        "command_id": "cmd_retry_task",
        "project_id": proj_id,
        "title": "Preserve Standard Plan Stage",
        "kind": "standard"
    });
    let created = assert_idempotent_post(
        client,
        &format!("{base}/api/v1/tasks"),
        &task_body,
        201,
        200,
        None,
    )
    .await;
    let task_id = created["id"].as_str().unwrap().to_string();

    assert_idempotent_post(
        client,
        &format!("{base}/api/v1/tasks/{task_id}/move"),
        &json!({ "command_id": "cmd_retry_move", "stage": "ready", "actor": "human" }),
        200,
        200,
        Some(&json!({ "command_id": "cmd_retry_move", "stage": "done", "actor": "human" })),
    )
    .await;

    let (clar_status, _) = post_json(
        client,
        format!("{base}/api/v1/tasks/{task_id}/clarify"),
        &json!({ "clarification": "Confirmed scope" }),
    )
    .await;
    assert_eq!(clar_status, 200);

    let plan_json = assert_idempotent_post(
        client,
        &format!("{base}/api/v1/tasks/{task_id}/plan"),
        &json!({ "command_id": "cmd_retry_plan", "plan_text": "1. Edit\n2. Verify", "approve": true }),
        200,
        200,
        None,
    )
    .await;
    assert_eq!(plan_json["workflow_stage"], "implementation");

    let (st_status, start_json) = post_json(
        client,
        format!("{base}/api/v1/tasks/{task_id}/start"),
        &json!({ "prompt": "implement" }),
    )
    .await;
    assert_eq!(st_status, 201);
    assert_eq!(start_json["task"]["workflow_stage"], "implementation");
    assert_eq!(start_json["run"]["workflow_stage"], "implementation");
    let run_id = start_json["run"]["id"].as_str().unwrap().to_string();
    let wt_path = start_json["run"]["worktree_path"].as_str().unwrap();

    let detail: Value = client
        .get(format!("{base}/api/v1/tasks/{task_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(detail["session_revision"].as_u64().unwrap_or(0) >= 1);

    fs::write(Path::new(wt_path).join("change.txt"), "ok\n").expect("write worktree file");
    let (ver_status, ver_json) = post_json(
        client,
        format!("{base}/api/v1/runs/{run_id}/verify"),
        &json!({ "task_id": task_id }),
    )
    .await;
    assert_eq!(ver_status, 200);
    assert_eq!(ver_json["verdict"], "pass");
    (task_id, run_id)
}

async fn assert_attempt_and_decision_retries(
    client: &Client,
    base: &str,
    task_id: &str,
    run_id: &str,
) {
    assert_idempotent_post(
        client,
        &format!("{base}/api/v1/runs/{run_id}/attempts"),
        &json!({
            "command_id": "cmd_retry_att",
            "workflow_stage": "implementation",
            "outcome": { "status": "succeeded" },
            "cost_micro_usd": 1200
        }),
        201,
        200,
        None,
    )
    .await;

    assert_idempotent_post(
        client,
        &format!("{base}/api/v1/tasks/{task_id}/reject"),
        &json!({ "command_id": "cmd_retry_rej", "reason": "Add edge case check" }),
        200,
        200,
        Some(&json!({ "command_id": "cmd_retry_rej", "reason": "Mutated reason" })),
    )
    .await;

    let (ver2_status, ver2_json) = post_json(
        client,
        format!("{base}/api/v1/runs/{run_id}/verify"),
        &json!({ "task_id": task_id }),
    )
    .await;
    assert_eq!(ver2_status, 200);
    assert_eq!(ver2_json["verdict"], "pass");

    assert_idempotent_post(
        client,
        &format!("{base}/api/v1/tasks/{task_id}/accept"),
        &json!({ "command_id": "cmd_retry_acc", "comment": "Ship it" }),
        200,
        200,
        Some(&json!({ "command_id": "cmd_retry_acc", "comment": "Different note" })),
    )
    .await;
}

#[tokio::test]
async fn idempotent_retries_and_standard_plan_workflow_hold_over_http() {
    let temp = TempDir::new().expect("tempdir");
    let (client, base, _state, repo_dir, server) = start_server(&temp).await;
    let proj_id = assert_project_idempotency(&client, &base, &repo_dir).await;
    let (task_id, run_id) = assert_task_plan_and_live_verify(&client, &base, &proj_id).await;
    assert_attempt_and_decision_retries(&client, &base, &task_id, &run_id).await;
    server.abort();
}

#[tokio::test]
async fn concurrent_scheduler_admit_once_serializes_worktrees() {
    let temp = TempDir::new().expect("tempdir");
    let (client, base, state, repo_dir, server) = start_server(&temp).await;
    let proj_id = assert_project_idempotency(&client, &base, &repo_dir).await;

    let first_task_id = client
        .post(format!("{base}/api/v1/tasks"))
        .json(&json!({ "project_id": proj_id, "title": "Conc A", "kind": "quick_fix" }))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let second_task_id = client
        .post(format!("{base}/api/v1/tasks"))
        .json(&json!({ "project_id": proj_id, "title": "Conc B", "kind": "quick_fix" }))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut conn = state.get_connection().expect("conn");
    for (idx, tid) in [first_task_id, second_task_id].into_iter().enumerate() {
        RunQueries::enqueue_task_run(
            &mut conn,
            &runlark_local::EnqueueTaskRunParams {
                command_id: runlark_domain::CommandId::from(format!("cmd_conc_{idx}")),
                task_id: runlark_domain::TaskId::from(tid),
                session_id: runlark_domain::SessionId::from(format!("sess_conc_{idx}")),
                run_id: runlark_domain::RunId::from(format!("run_conc_{idx}")),
                prompt: "work".to_string(),
                workflow_digest: None,
                base_sha: None,
                expected_revision: None,
            },
            Timestamp(100_000 + u64::try_from(idx).unwrap_or(0)),
        )
        .expect("enqueue conc");
    }
    drop(conn);

    let s_one = Scheduler::new(state.clone());
    let s_two = Scheduler::new(state.clone());
    let (res_a, res_b) = tokio::join!(s_one.admit_once(), s_two.admit_once());
    let admitted_total = res_a.expect("sched a").len() + res_b.expect("sched b").len();
    assert_eq!(admitted_total, 2);

    let conn = state.get_connection().expect("conn");
    for idx in 0..2 {
        let r = SessionStore::get_run(
            &conn,
            &runlark_domain::RunId::from(format!("run_conc_{idx}")),
        )
        .unwrap()
        .unwrap();
        assert_eq!(r.status, RunStatus::Running);
        assert!(!r.worktree_path.is_empty());
    }
    server.abort();
}
