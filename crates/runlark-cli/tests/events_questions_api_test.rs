use reqwest::Client;
use runlark_cli::{AppState, StreamEventNotification, build_router};
use runlark_domain::{
    CommandId, QuestionId, Revision, RunId, SendPromptCommand, Session, SessionCommand, SessionId,
    SessionStatus, TaskId, Timestamp,
};
use runlark_local::{LocalStore, SessionStore, initialize_schema, open_sqlite};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tempfile::TempDir;

async fn spawn_api_server(
    db_path: &std::path::Path,
) -> (AppState, String, tokio::task::JoinHandle<()>) {
    let conn = open_sqlite(db_path).expect("open db");
    initialize_schema(&conn).expect("init schema");
    drop(conn);

    let state = AppState::new(db_path.to_path_buf());
    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (state, format!("http://{addr}"), handle)
}

#[tokio::test]
async fn test_list_questions_filters_both_run_id_and_session_id() {
    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("questions_filter.db");
    let (state, base_url, handle) = spawn_api_server(&db_path).await;
    let client = Client::new();

    for (sess, run, qid) in [("sess_a", "run_a", "q_a"), ("sess_b", "run_b", "q_b")] {
        let mut conn = state.get_connection().expect("conn");
        SessionStore::create_session(
            &conn,
            &Session {
                id: SessionId::from(sess),
                task_id: TaskId::from("task_1"),
                cwd: temp.path().display().to_string(),
                status: SessionStatus::Created,
                current_revision: Revision(1),
                active_run_id: None,
                created_at: Timestamp(1000),
                updated_at: Timestamp(1000),
            },
        )
        .expect("create session");
        LocalStore::apply_command(
            &mut conn,
            &SessionCommand::SendPrompt(SendPromptCommand {
                command_id: CommandId::from(format!("cmd_{sess}")),
                session_id: SessionId::from(sess),
                run_id: RunId::from(run),
                prompt: "Ask".to_string(),
                expected_revision: Revision(1),
                created_at: Timestamp(1010),
            }),
            Timestamp(1010),
        )
        .expect("send prompt");
        LocalStore::record_user_question(
            &mut conn,
            &SessionId::from(sess),
            &RunId::from(run),
            &QuestionId::from(qid),
            "Approve?",
            &["Yes".to_string()],
            Timestamp(1020),
        )
        .expect("record question");
    }

    // Mismatched run_id + session_id must return empty rather than ignoring session_id
    let mismatch: Vec<Value> = client
        .get(format!(
            "{base_url}/api/v1/questions?run_id=run_a&session_id=sess_b"
        ))
        .send()
        .await
        .expect("get mismatch")
        .json()
        .await
        .expect("json");
    assert!(mismatch.is_empty());

    // Matching run_id + session_id returns the expected question
    let matched: Vec<Value> = client
        .get(format!(
            "{base_url}/api/v1/questions?run_id=run_a&session_id=sess_a"
        ))
        .send()
        .await
        .expect("get match")
        .json()
        .await
        .expect("json");
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0]["question_id"], "q_a");

    state.signal_shutdown();
    handle.abort();
}

#[tokio::test]
async fn test_get_events_nonexistent_run_returns_not_found_immediately() {
    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("events_missing.db");
    let (state, base_url, handle) = spawn_api_server(&db_path).await;
    let client = Client::new();

    let started = Instant::now();
    let resp = client
        .get(format!(
            "{base_url}/api/v1/runs/missing_run_id/events?wait_ms=10000"
        ))
        .send()
        .await
        .expect("get missing run");
    let elapsed = started.elapsed();

    assert_eq!(resp.status().as_u16(), 404);
    assert!(
        elapsed < Duration::from_secs(2),
        "nonexistent run must return 404 immediately (elapsed {elapsed:?})"
    );

    state.signal_shutdown();
    handle.abort();
}

#[tokio::test]
async fn test_events_long_poll_breaks_immediately_on_broadcast_lag() {
    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("events_lag.db");
    let (state, base_url, handle) = spawn_api_server(&db_path).await;
    let client = Client::new();

    {
        let mut conn = state.get_connection().expect("conn");
        SessionStore::create_session(
            &conn,
            &Session {
                id: SessionId::from("sess_lag"),
                task_id: TaskId::from("task_lag"),
                cwd: temp.path().display().to_string(),
                status: SessionStatus::Created,
                current_revision: Revision(1),
                active_run_id: None,
                created_at: Timestamp(1000),
                updated_at: Timestamp(1000),
            },
        )
        .expect("create session");
        LocalStore::apply_command(
            &mut conn,
            &SessionCommand::SendPrompt(SendPromptCommand {
                command_id: CommandId::from("cmd_lag_start"),
                session_id: SessionId::from("sess_lag"),
                run_id: RunId::from("run_lag"),
                prompt: "Initial".to_string(),
                expected_revision: Revision(1),
                created_at: Timestamp(1010),
            }),
            Timestamp(1010),
        )
        .expect("send prompt");
    }

    let started = Instant::now();
    let poll_client = client.clone();
    let poll_url = format!("{base_url}/api/v1/runs/run_lag/events?after_seq=0&wait_ms=10000");
    let poll_task = tokio::spawn(async move {
        poll_client
            .get(&poll_url)
            .send()
            .await
            .expect("poll")
            .json::<Value>()
            .await
            .expect("json")
    });

    tokio::time::sleep(Duration::from_millis(60)).await;
    // Persist the target run's next event, then overflow the 256-slot broadcast buffer with
    // unrelated notifications so the waiter receives RecvError::Lagged.
    let res = client
        .post(format!("{base_url}/api/v1/dsh/events"))
        .json(&json!({
            "TurnEnded": {
                "session_id": "sess_lag",
                "run_id": "run_lag",
                "outcome": "Completed"
            }
        }))
        .send()
        .await
        .expect("post turn end");
    assert_eq!(res.status().as_u16(), 200);

    for seq in 0..300_u64 {
        let _ = state.event_tx.send(StreamEventNotification {
            session_id: "other_sess".to_string(),
            run_id: "other_run".to_string(),
            seq,
        });
    }

    let events = poll_task.await.expect("poll task");
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "long-poll must break on Lagged instead of waiting 10s (elapsed {elapsed:?})"
    );
    let arr = events.as_array().expect("events array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["event_type"], "RunCompleted");

    state.signal_shutdown();
    handle.abort();
}
