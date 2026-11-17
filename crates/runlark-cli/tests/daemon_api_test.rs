use reqwest::Client;
use runlark_cli::{AppState, LocalBridgeEffector, OutboxConfig, OutboxDispatcher, build_router};
use runlark_local::{CommandStore, initialize_schema, open_sqlite};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::watch;

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
    fs::write(dir.join("README.md"), "# Daemon API Test\n").expect("write");
    run(&["add", "README.md"]);
    run(&["commit", "-m", "init"]);
}

async fn spawn_test_server(
    db_path: &Path,
) -> (String, watch::Sender<bool>, tokio::task::JoinHandle<()>) {
    let conn = open_sqlite(db_path).expect("open db");
    initialize_schema(&conn).expect("init schema");
    drop(conn);

    let state = AppState::new(db_path.to_path_buf());
    let dispatcher = Arc::new(OutboxDispatcher::new(
        db_path.to_path_buf(),
        Arc::new(LocalBridgeEffector),
        OutboxConfig {
            poll_interval: Duration::from_millis(50),
            retry_backoff_ms: 0,
            cleanup_worktree_on_archive: false,
        },
    ));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(dispatcher.run_loop(state.outbox_notify.clone(), shutdown_rx));

    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");

    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (format!("http://{addr}"), shutdown_tx, handle)
}

async fn post_json(client: &Client, url: &str, body: &Value) -> (u16, Value) {
    let resp = client.post(url).json(body).send().await.expect("post json");
    let status = resp.status().as_u16();
    let json = resp.json().await.expect("parse json");
    (status, json)
}

async fn create_session_ok(client: &Client, base_url: &str, session_id: &str, cwd: &Path) {
    let (status, body) = post_json(
        client,
        &format!("{base_url}/api/v1/sessions"),
        &json!({
            "session_id": session_id,
            "task_id": "task_api_1",
            "cwd": cwd.to_string_lossy(),
        }),
    )
    .await;
    assert_eq!(status, 201);
    assert_eq!(body["current_revision"], 1);
}

#[tokio::test]
async fn test_daemon_session_and_command_error_mapping() {
    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("err.db");
    let (base_url, shutdown_tx, server_handle) = spawn_test_server(&db_path).await;
    let client = Client::new();

    create_session_ok(&client, &base_url, "sess_err", temp.path()).await;

    let (dup_status, _) = post_json(
        &client,
        &format!("{base_url}/api/v1/sessions"),
        &json!({
            "session_id": "sess_err",
            "task_id": "task_api_1",
            "cwd": temp.path().to_string_lossy(),
        }),
    )
    .await;
    assert_eq!(dup_status, 409);

    let (missing_status, _) = post_json(
        &client,
        &format!("{base_url}/api/v1/commands"),
        &json!({
            "type": "SendPrompt",
            "payload": {
                "command_id": "cmd_missing",
                "session_id": "sess_nope",
                "run_id": "run_nope",
                "prompt": "Hi",
                "expected_revision": 1,
                "created_at": 1000
            }
        }),
    )
    .await;
    assert_eq!(missing_status, 404);

    let (bad_rev_status, bad_rev_body) = post_json(
        &client,
        &format!("{base_url}/api/v1/commands"),
        &json!({
            "type": "SendPrompt",
            "payload": {
                "command_id": "cmd_bad_rev",
                "session_id": "sess_err",
                "run_id": "run_err",
                "prompt": "Hi",
                "expected_revision": 99,
                "created_at": 1000
            }
        }),
    )
    .await;
    assert_eq!(bad_rev_status, 409);
    assert_eq!(bad_rev_body["error"], "revision_mismatch");
    assert_eq!(bad_rev_body["actual_revision"], 1);

    let _ = shutdown_tx.send(true);
    server_handle.abort();
}

#[tokio::test]
async fn test_daemon_command_worktree_and_idempotency() {
    let temp = TempDir::new().expect("temp dir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo");
    init_git_repo(&repo_dir);

    let db_path = temp.path().join("runlark.db");
    let (base_url, shutdown_tx, server_handle) = spawn_test_server(&db_path).await;
    let client = Client::new();

    create_session_ok(&client, &base_url, "sess_api_1", &repo_dir).await;

    let prompt_payload = json!({
        "type": "SendPrompt",
        "payload": {
            "command_id": "cmd_prompt_1",
            "session_id": "sess_api_1",
            "run_id": "run_api_1",
            "prompt": "Implement feature",
            "expected_revision": 1,
            "created_at": 1010
        }
    });
    let (send_status, send_body) = post_json(
        &client,
        &format!("{base_url}/api/v1/commands"),
        &prompt_payload,
    )
    .await;
    assert_eq!(send_status, 201);
    assert_eq!(send_body["current_revision"], 2);
    assert_eq!(send_body["run_status"], "running");

    let (replay_status, _) = post_json(
        &client,
        &format!("{base_url}/api/v1/commands"),
        &prompt_payload,
    )
    .await;
    assert_eq!(replay_status, 200);

    let (conflict_status, _) = post_json(
        &client,
        &format!("{base_url}/api/v1/commands"),
        &json!({
            "type": "SendPrompt",
            "payload": {
                "command_id": "cmd_prompt_1",
                "session_id": "sess_api_1",
                "run_id": "run_api_1",
                "prompt": "Mutated prompt",
                "expected_revision": 1,
                "created_at": 1010
            }
        }),
    )
    .await;
    assert_eq!(conflict_status, 409);

    let mut acked = false;
    for _ in 0..40 {
        let conn = open_sqlite(&db_path).expect("open db");
        if let Some(cmd) =
            CommandStore::get_command(&conn, &runlark_domain::CommandId::from("cmd_prompt_1"))
                .expect("get cmd")
            && cmd.status == "acked"
        {
            acked = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(acked, "background outbox must ack SendPrompt");

    let run_resp: Value = client
        .get(format!("{base_url}/api/v1/runs/run_api_1"))
        .send()
        .await
        .expect("get run")
        .json()
        .await
        .expect("json");
    assert!(
        run_resp["worktree_path"]
            .as_str()
            .expect("wt path")
            .ends_with(".runlark/worktrees/run_api_1")
    );

    let _ = shutdown_tx.send(true);
    server_handle.abort();
}

#[tokio::test]
async fn test_daemon_questions_and_answer_lifecycle() {
    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("q.db");
    let (base_url, shutdown_tx, server_handle) = spawn_test_server(&db_path).await;
    let client = Client::new();

    create_session_ok(&client, &base_url, "sess_q_1", temp.path()).await;

    let (start_status, _) = post_json(
        &client,
        &format!("{base_url}/api/v1/commands"),
        &json!({
            "type": "SendPrompt",
            "payload": {
                "command_id": "cmd_q_start",
                "session_id": "sess_q_1",
                "run_id": "run_q_1",
                "prompt": "Deploy?",
                "expected_revision": 1,
                "created_at": 1000
            }
        }),
    )
    .await;
    assert_eq!(start_status, 201);

    let (ask_status, ask_body) = post_json(
        &client,
        &format!("{base_url}/api/v1/dsh/events"),
        &json!({
            "UserQuestionAsked": {
                "session_id": "sess_q_1",
                "run_id": "run_q_1",
                "question_id": "q_api_1",
                "prompt": "Proceed with deploy?",
                "options": ["yes", "no"]
            }
        }),
    )
    .await;
    assert_eq!(ask_status, 200);
    assert_eq!(ask_body["run_status"], "waiting_for_approval");

    let sess_detail: Value = client
        .get(format!("{base_url}/api/v1/sessions/sess_q_1"))
        .send()
        .await
        .expect("get session")
        .json()
        .await
        .expect("json");
    assert_eq!(sess_detail["pending_question"]["question_id"], "q_api_1");

    let (ans_status, ans_body) = post_json(
        &client,
        &format!("{base_url}/api/v1/questions/q_api_1/answer"),
        &json!({
            "command_id": "cmd_ans_1",
            "answer": "yes",
            "expected_revision": 3
        }),
    )
    .await;
    assert_eq!(ans_status, 201);
    assert_eq!(ans_body["run_status"], "running");

    let _ = shutdown_tx.send(true);
    server_handle.abort();
}

#[tokio::test]
async fn test_daemon_stop_turn_end_and_archive() {
    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("stop.db");
    let (base_url, shutdown_tx, server_handle) = spawn_test_server(&db_path).await;
    let client = Client::new();

    create_session_ok(&client, &base_url, "sess_stop_1", temp.path()).await;

    let (start_status, _) = post_json(
        &client,
        &format!("{base_url}/api/v1/commands"),
        &json!({
            "type": "SendPrompt",
            "payload": {
                "command_id": "cmd_s_start",
                "session_id": "sess_stop_1",
                "run_id": "run_stop_1",
                "prompt": "Run work",
                "expected_revision": 1,
                "created_at": 1000
            }
        }),
    )
    .await;
    assert_eq!(start_status, 201);

    let (stop_status, stop_body) = post_json(
        &client,
        &format!("{base_url}/api/v1/commands"),
        &json!({
            "type": "StopRun",
            "payload": {
                "command_id": "cmd_stop_1",
                "session_id": "sess_stop_1",
                "run_id": "run_stop_1",
                "reason": "user abort",
                "expected_revision": 2,
                "created_at": 1020
            }
        }),
    )
    .await;
    assert_eq!(stop_status, 201);
    assert_eq!(stop_body["run_status"], "cancel_requested");

    let (end_status, end_body) = post_json(
        &client,
        &format!("{base_url}/api/v1/dsh/events"),
        &json!({
            "TurnEnded": {
                "session_id": "sess_stop_1",
                "run_id": "run_stop_1",
                "outcome": "Completed"
            }
        }),
    )
    .await;
    assert_eq!(end_status, 200);
    assert_eq!(end_body["run_status"], "stopped");
    assert_eq!(end_body["current_revision"], 4);

    let (arch_status, _) = post_json(
        &client,
        &format!("{base_url}/api/v1/commands"),
        &json!({
            "type": "ArchiveSession",
            "payload": {
                "command_id": "cmd_arch_1",
                "session_id": "sess_stop_1",
                "run_id": "run_stop_1",
                "expected_revision": 4,
                "created_at": 1030
            }
        }),
    )
    .await;
    assert_eq!(arch_status, 201);

    let events: Value = client
        .get(format!("{base_url}/api/v1/sessions/sess_stop_1/events"))
        .send()
        .await
        .expect("events")
        .json()
        .await
        .expect("json");
    assert_eq!(events.as_array().expect("array").len(), 4);

    let _ = shutdown_tx.send(true);
    server_handle.abort();
}

#[tokio::test]
async fn test_daemon_session_enrichment_and_restart_waiting() {
    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("ui_state.db");
    let (base_url, shutdown_tx, server_handle) = spawn_test_server(&db_path).await;
    let client = Client::new();

    create_session_ok(&client, &base_url, "sess_ui_1", temp.path()).await;

    let (send_status, _) = post_json(
        &client,
        &format!("{base_url}/api/v1/commands"),
        &json!({
            "type": "SendPrompt",
            "payload": {
                "command_id": "cmd_ui_send_1",
                "session_id": "sess_ui_1",
                "run_id": "run_ui_1",
                "prompt": "Ask user before proceeding",
                "expected_revision": 1,
                "created_at": 1000
            }
        }),
    )
    .await;
    assert_eq!(send_status, 201);

    let (q_status, _) = post_json(
        &client,
        &format!("{base_url}/api/v1/dsh/events"),
        &json!({
            "UserQuestionAsked": {
                "session_id": "sess_ui_1",
                "run_id": "run_ui_1",
                "question_id": "q_ui_1",
                "prompt": "Approve migration?",
                "options": ["Approve", "Reject"]
            }
        }),
    )
    .await;
    assert_eq!(q_status, 200);

    // Wait for outbox dispatcher to ack cmd_ui_send_1 before simulating restart
    let mut acked = false;
    for _ in 0..40 {
        let conn = open_sqlite(&db_path).expect("open");
        if CommandStore::get_command(&conn, &runlark_domain::CommandId::from("cmd_ui_send_1"))
            .expect("get cmd")
            .is_some_and(|c| c.status == "acked")
        {
            acked = true;
            break;
        }
        drop(conn);
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    assert!(acked, "outbox should ack cmd_ui_send_1 before restart");

    // Simulate daemon restart against the same SQLite DB
    let _ = shutdown_tx.send(true);
    server_handle.abort();

    {
        let mut conn = open_sqlite(&db_path).expect("reopen");
        let inflight = runlark_local::ReconciliationEngine::reconcile_on_startup(
            &mut conn,
            runlark_domain::Timestamp(2000),
        )
        .expect("reconcile");
        assert_eq!(inflight.waiting_runs.len(), 1);
    }

    let (base_url_2, shutdown_tx_2, server_handle_2) = spawn_test_server(&db_path).await;
    let restored: Value = client
        .get(format!("{base_url_2}/api/v1/sessions/sess_ui_1"))
        .send()
        .await
        .expect("get restored session")
        .json()
        .await
        .expect("json");
    assert_eq!(restored["active_run_status"], "waiting_for_approval");
    assert_eq!(restored["latest_run_id"], "run_ui_1");
    assert_eq!(restored["latest_run_status"], "waiting_for_approval");
    assert_eq!(restored["pending_question"]["question_id"], "q_ui_1");
    assert_eq!(restored["dispatched_commands_count"], 0);

    let _ = shutdown_tx_2.send(true);
    server_handle_2.abort();
}

#[tokio::test]
async fn test_daemon_session_long_poll_and_terminal_latest_run() {
    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("ui_poll.db");
    let (base_url, shutdown_tx, server_handle) = spawn_test_server(&db_path).await;
    let client = Client::new();

    create_session_ok(&client, &base_url, "sess_ui_2", temp.path()).await;
    let (send_status, _) = post_json(
        &client,
        &format!("{base_url}/api/v1/commands"),
        &json!({
            "type": "SendPrompt",
            "payload": {
                "command_id": "cmd_ui_send_2",
                "session_id": "sess_ui_2",
                "run_id": "run_ui_2",
                "prompt": "Need approval",
                "expected_revision": 1,
                "created_at": 1000
            }
        }),
    )
    .await;
    assert_eq!(send_status, 201);

    let (q_status, _) = post_json(
        &client,
        &format!("{base_url}/api/v1/dsh/events"),
        &json!({
            "UserQuestionAsked": {
                "session_id": "sess_ui_2",
                "run_id": "run_ui_2",
                "question_id": "q_ui_2",
                "prompt": "Continue?",
                "options": ["Yes", "No"]
            }
        }),
    )
    .await;
    assert_eq!(q_status, 200);

    let poll_client = client.clone();
    let poll_url = format!("{base_url}/api/v1/sessions/sess_ui_2/events?after_seq=1&wait_ms=4000");
    let poll_handle = tokio::spawn(async move {
        poll_client
            .get(&poll_url)
            .send()
            .await
            .expect("poll")
            .json::<Value>()
            .await
            .expect("poll json")
    });

    tokio::time::sleep(Duration::from_millis(80)).await;
    let (ans_status, ans_body) = post_json(
        &client,
        &format!("{base_url}/api/v1/questions/q_ui_2/answer"),
        &json!({
            "command_id": "cmd_ui_ans_2",
            "answer": "Yes",
            "expected_revision": 3
        }),
    )
    .await;
    assert_eq!(ans_status, 201);
    assert_eq!(ans_body["run_status"], "running");

    let polled = poll_handle.await.expect("poll task");
    let polled_arr = polled.as_array().expect("polled array");
    assert_eq!(polled_arr.len(), 1);
    assert_eq!(polled_arr[0]["event_type"], "QuestionAnswered");
    assert_eq!(polled_arr[0]["run_id"], "run_ui_2");
    assert_eq!(polled_arr[0]["seq"], 2);

    let (end_status, _) = post_json(
        &client,
        &format!("{base_url}/api/v1/dsh/events"),
        &json!({
            "TurnEnded": {
                "session_id": "sess_ui_2",
                "run_id": "run_ui_2",
                "outcome": "Completed"
            }
        }),
    )
    .await;
    assert_eq!(end_status, 200);

    let finished: Value = client
        .get(format!("{base_url}/api/v1/sessions/sess_ui_2"))
        .send()
        .await
        .expect("get finished session")
        .json()
        .await
        .expect("json");
    assert!(finished["active_run_id"].is_null());
    assert_eq!(finished["latest_run_id"], "run_ui_2");
    assert_eq!(finished["latest_run_status"], "completed");

    let _ = shutdown_tx.send(true);
    server_handle.abort();
}
