use futures_util::{SinkExt, StreamExt};
use runlark_cli::{AppState, DshSubscriber, DshWireEvent, process_dsh_wire_event};
use runlark_domain::{
    CommandId, QuestionId, Revision, RunId, RunStatus, SendPromptCommand, Session, SessionCommand,
    SessionId, SessionStatus, StopRunCommand, TaskId, Timestamp,
};
use runlark_local::{
    CommandStore, DshCursorStore, EventStore, LocalStore, QuestionStore, ReconciliationEngine,
    SessionStore, initialize_schema, open_sqlite,
};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Notify, watch};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

async fn spawn_mock_dsh_mux(
    expected_token: &str,
    connect_count: Arc<AtomicUsize>,
    ws_frames: Arc<Vec<Value>>,
    send_live_turn_end: Arc<Notify>,
) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock dsh");
    let addr = listener.local_addr().expect("local addr");
    let token = expected_token.to_string();

    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let token = token.clone();
            let connect_count = Arc::clone(&connect_count);
            let ws_frames = Arc::clone(&ws_frames);
            let send_live_turn_end = Arc::clone(&send_live_turn_end);

            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = stream.peek(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).into_owned();

                if head.starts_with("GET /connect") {
                    let mut drain = vec![0u8; n];
                    let _ = stream.read(&mut drain).await;
                    if head.contains(&format!("token={token}")) {
                        connect_count.fetch_add(1, Ordering::SeqCst);
                        let resp = concat!(
                            "HTTP/1.1 302 Found\r\n",
                            "Set-Cookie: dsh-auth-test=secret_cookie; Path=/; HttpOnly\r\n",
                            "Location: ./api/remote.mux\r\n",
                            "Content-Length: 0\r\n\r\n"
                        );
                        let _ = stream.write_all(resp.as_bytes()).await;
                    } else {
                        let _ = stream
                            .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n")
                            .await;
                    }
                    return;
                }

                assert!(
                    head.contains("dsh-auth-test=secret_cookie"),
                    "WS upgrade missing auth cookie: {head}"
                );
                let ws_stream = accept_async(stream).await.expect("accept ws");
                let (mut write, mut read) = ws_stream.split();
                while let Some(Ok(Message::Text(txt))) = read.next().await {
                    let open: Value = serde_json::from_str(&txt).expect("parse mux open");
                    if open["type"] == "open" && open["endpoint"] == "session/follow" {
                        let stream_id = open["streamId"].as_str().expect("streamId").to_string();
                        let snapshot = json!({
                            "type": "item",
                            "streamId": stream_id,
                            "value": {
                                "type": "snapshot",
                                "records": ws_frames
                                    .iter()
                                    .map(|ev| json!({ "type": "event", "event": ev }))
                                    .collect::<Vec<_>>(),
                            }
                        });
                        let _ = write.send(Message::Text(snapshot.to_string().into())).await;

                        send_live_turn_end.notified().await;
                        let live_end = json!({
                            "type": "item",
                            "streamId": stream_id,
                            "value": {
                                "type": "event",
                                "event": {
                                    "type": "turn/end",
                                    "seq": 4,
                                    "time": 2000,
                                    "data": { "turn": 1, "reason": { "kind": "aborted", "reason": { "kind": "user" } } }
                                }
                            }
                        });
                        let _ = write.send(Message::Text(live_end.to_string().into())).await;
                        break;
                    }
                }
            });
        }
    });

    addr
}

fn seed_dispatched_run(db_path: &Path, cwd: &Path, sess_id: &str, run_id: &str, cmd_id: &str) {
    let mut conn = open_sqlite(db_path).expect("open db");
    initialize_schema(&conn).expect("init schema");
    let sess = Session {
        id: SessionId::from(sess_id),
        task_id: TaskId::from("task_sub"),
        cwd: cwd.to_string_lossy().into_owned(),
        status: SessionStatus::Created,
        current_revision: Revision(1),
        active_run_id: None,
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
    };
    SessionStore::create_session(&conn, &sess).expect("create session");

    let send_cmd = SessionCommand::SendPrompt(SendPromptCommand {
        command_id: CommandId::from(cmd_id),
        session_id: sess.id,
        run_id: RunId::from(run_id),
        prompt: "Ask question then stop".to_string(),
        expected_revision: Revision(1),
        created_at: Timestamp(1010),
    });
    LocalStore::apply_command(&mut conn, &send_cmd, Timestamp(1010)).expect("send");
    CommandStore::mark_dispatched(&conn, send_cmd.command_id(), Timestamp(1015))
        .expect("mark dispatched");
}

fn sample_frames(cmd_id: &str) -> Arc<Vec<Value>> {
    Arc::new(vec![
        json!({ "type": "turn/start", "seq": 1, "time": 1100, "data": { "turn": 1 } }),
        json!({
            "type": "user/message",
            "seq": 2,
            "time": 1110,
            "data": {
                "id": "msg_1",
                "source": { "kind": "user", "rpcId": cmd_id },
                "content": [{ "type": "text", "text": "Ask question then stop" }]
            }
        }),
        json!({
            "type": "tool/call",
            "seq": 3,
            "time": 1120,
            "data": {
                "turn": 1,
                "step": 1,
                "callId": "q_sub_1",
                "name": "ask_user_question",
                "arguments": "{\"questions\":[{\"id\":\"q1\",\"question\":\"Continue?\",\"options\":[{\"label\":\"Yes\"}]}]}"
            }
        }),
    ])
}

async fn wait_for_run_status(db_path: &Path, run_id: &str, expected: RunStatus) -> bool {
    for _ in 0..40 {
        let conn = open_sqlite(db_path).expect("open db");
        if let Some(run) = SessionStore::get_run(&conn, &RunId::from(run_id)).expect("get run")
            && run.status == expected
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

#[tokio::test]
async fn test_dsh_subscriber_authenticates_and_transitions_lifecycle() {
    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("sub.db");
    seed_dispatched_run(
        &db_path,
        temp.path(),
        "sess_sub_1",
        "run_sub_1",
        "cmd_sub_send",
    );

    let connect_count = Arc::new(AtomicUsize::new(0));
    let send_live_turn_end = Arc::new(Notify::new());
    let dsh_addr = spawn_mock_dsh_mux(
        "tok_test_123",
        Arc::clone(&connect_count),
        sample_frames("cmd_sub_send"),
        Arc::clone(&send_live_turn_end),
    )
    .await;

    let state = AppState::new(db_path.clone());
    let (stop_tx, stop_rx) = watch::channel(false);
    let subscriber = DshSubscriber::new(
        state.clone(),
        &format!("http://{dsh_addr}/?token=tok_test_123"),
        None,
    );
    let sub_task = tokio::spawn(subscriber.run_loop(state.dsh_notify.clone(), stop_rx));

    assert!(wait_for_run_status(&db_path, "run_sub_1", RunStatus::WaitingForApproval).await);
    assert_eq!(connect_count.load(Ordering::SeqCst), 1);

    {
        let mut conn = open_sqlite(&db_path).expect("open db");
        let cmd = CommandStore::get_command(&conn, &CommandId::from("cmd_sub_send"))
            .expect("get cmd")
            .expect("exists");
        assert_eq!(cmd.status, "acked");
        assert_eq!(
            DshCursorStore::get_cursor(&conn, &SessionId::from("sess_sub_1")).expect("cursor"),
            Some(3)
        );
        let q = QuestionStore::get_question(&conn, &QuestionId::from("q_sub_1"))
            .expect("get q")
            .expect("exists");
        assert_eq!(q.prompt, "Continue?");

        let sess = SessionStore::get_session(&conn, &SessionId::from("sess_sub_1"))
            .expect("get sess")
            .expect("exists");
        let stop_cmd = SessionCommand::StopRun(StopRunCommand {
            command_id: CommandId::from("cmd_sub_stop"),
            session_id: sess.id,
            run_id: RunId::from("run_sub_1"),
            reason: Some("user stop".to_string()),
            expected_revision: sess.current_revision,
            created_at: Timestamp(1500),
        });
        LocalStore::apply_command(&mut conn, &stop_cmd, Timestamp(1500)).expect("stop cmd");
        let run = SessionStore::get_run(&conn, &RunId::from("run_sub_1"))
            .expect("get run")
            .expect("exists");
        assert_eq!(run.status, RunStatus::CancelRequested);
    }

    send_live_turn_end.notify_one();
    assert!(wait_for_run_status(&db_path, "run_sub_1", RunStatus::Stopped).await);

    let _ = stop_tx.send(true);
    let _ = sub_task.await;
}

#[tokio::test]
async fn test_dsh_subscriber_reconnect_deduplicates_without_duplicate_runs() {
    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("dedup.db");
    seed_dispatched_run(&db_path, temp.path(), "sess_dup", "run_dup", "cmd_dup_send");

    let connect_count = Arc::new(AtomicUsize::new(0));
    let send_live_turn_end = Arc::new(Notify::new());
    let dsh_addr = spawn_mock_dsh_mux(
        "tok_dup",
        Arc::clone(&connect_count),
        sample_frames("cmd_dup_send"),
        Arc::clone(&send_live_turn_end),
    )
    .await;

    let state1 = AppState::new(db_path.clone());
    let (tx1, rx1) = watch::channel(false);
    let sub1 = DshSubscriber::new(
        state1.clone(),
        &format!("http://{dsh_addr}/?token=tok_dup"),
        None,
    );
    let task1 = tokio::spawn(sub1.run_loop(state1.dsh_notify.clone(), rx1));
    assert!(wait_for_run_status(&db_path, "run_dup", RunStatus::WaitingForApproval).await);
    send_live_turn_end.notify_one();
    assert!(wait_for_run_status(&db_path, "run_dup", RunStatus::Stopped).await);
    let _ = tx1.send(true);
    let _ = task1.await;

    let event_count_before = {
        let mut conn = open_sqlite(&db_path).expect("open db");
        let inflight = ReconciliationEngine::reconcile_on_startup(&mut conn, Timestamp(2500))
            .expect("reconcile");
        assert!(inflight.active_runs.is_empty());
        EventStore::get_events(&conn, &RunId::from("run_dup"), None)
            .expect("events")
            .len()
    };

    let state2 = AppState::new(db_path.clone());
    let (tx2, rx2) = watch::channel(false);
    let sub2 = DshSubscriber::new(
        state2.clone(),
        &format!("http://{dsh_addr}/?token=tok_dup"),
        None,
    );
    let task2 = tokio::spawn(sub2.run_loop(state2.dsh_notify.clone(), rx2));
    tokio::time::sleep(Duration::from_millis(120)).await;
    let _ = tx2.send(true);
    let _ = task2.await;

    let conn = open_sqlite(&db_path).expect("open db");
    let runs = SessionStore::list_runs_for_session(&conn, &SessionId::from("sess_dup"))
        .expect("list runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, RunStatus::Stopped);
    let event_count_after = EventStore::get_events(&conn, &RunId::from("run_dup"), None)
        .expect("events")
        .len();
    assert_eq!(event_count_after, event_count_before);
}

#[test]
fn test_synthetic_interrupted_turn_end_only_stops_when_cancel_requested() {
    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("repair.db");
    seed_dispatched_run(
        &db_path,
        temp.path(),
        "sess_repair",
        "run_repair",
        "cmd_repair_send",
    );

    let mut conn = open_sqlite(&db_path).expect("open db");
    let state = AppState::new(db_path);
    let sid = SessionId::from("sess_repair");
    let rid = RunId::from("run_repair");

    let synthetic_interrupted = DshWireEvent {
        event_type: "turn/end".to_string(),
        seq: 10,
        time: 2000,
        data: json!({ "turn": 1, "reason": { "kind": "interrupted" } }),
    };

    // Crash-repair turn/end(interrupted) on Running must record raw event without killing run.
    let emitted = process_dsh_wire_event(
        &mut conn,
        &state,
        &sid,
        Some("run_repair"),
        &synthetic_interrupted,
        Timestamp(2000),
    )
    .expect("process wire");
    assert_eq!(emitted.len(), 1);
    assert_eq!(emitted[0].event_type, "DshTurnEnded");

    let run = SessionStore::get_run(&conn, &rid)
        .expect("get run")
        .expect("exists");
    assert_eq!(run.status, RunStatus::Running);

    // Explicitly requested cancellation settles to Stopped when turn/end arrives.
    let sess = SessionStore::get_session(&conn, &sid)
        .expect("get sess")
        .expect("exists");
    let stop_cmd = SessionCommand::StopRun(StopRunCommand {
        command_id: CommandId::from("cmd_repair_stop"),
        session_id: sid.clone(),
        run_id: rid.clone(),
        reason: Some("user stop".to_string()),
        expected_revision: sess.current_revision,
        created_at: Timestamp(2100),
    });
    LocalStore::apply_command(&mut conn, &stop_cmd, Timestamp(2100)).expect("stop");

    let aborted_end = DshWireEvent {
        event_type: "turn/end".to_string(),
        seq: 11,
        time: 2200,
        data: json!({ "turn": 1, "reason": { "kind": "interrupted" } }),
    };
    process_dsh_wire_event(
        &mut conn,
        &state,
        &sid,
        Some("run_repair"),
        &aborted_end,
        Timestamp(2200),
    )
    .expect("process stop end");

    let stopped_run = SessionStore::get_run(&conn, &rid)
        .expect("get run")
        .expect("exists");
    assert_eq!(stopped_run.status, RunStatus::Stopped);
}

#[test]
fn test_empty_user_question_reply_leaves_question_pending_and_joins_parts() {
    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("empty_reply.db");
    seed_dispatched_run(
        &db_path,
        temp.path(),
        "sess_reply",
        "run_reply",
        "cmd_reply_send",
    );

    let mut conn = open_sqlite(&db_path).expect("open db");
    let state = AppState::new(db_path);
    let sid = SessionId::from("sess_reply");
    let rid = RunId::from("run_reply");

    let ask_event = DshWireEvent {
        event_type: "tool/call".to_string(),
        seq: 1,
        time: 2000,
        data: json!({
            "name": "ask_user_question",
            "callId": "q_reply_1",
            "arguments": r#"{"questions":[{"question":"Proceed?"}]}"#
        }),
    };
    process_dsh_wire_event(
        &mut conn,
        &state,
        &sid,
        Some("run_reply"),
        &ask_event,
        Timestamp(2000),
    )
    .expect("ask");

    // Empty / missing content must NOT answer the question
    let empty_reply = DshWireEvent {
        event_type: "user/message".to_string(),
        seq: 2,
        time: 2100,
        data: json!({
            "source": { "kind": "user-question-reply", "callId": "q_reply_1" },
            "content": [{ "type": "text", "text": "   " }]
        }),
    };
    process_dsh_wire_event(
        &mut conn,
        &state,
        &sid,
        Some("run_reply"),
        &empty_reply,
        Timestamp(2100),
    )
    .expect("empty reply");

    let still_pending = runlark_local::QuestionStore::get_question(
        &conn,
        &runlark_domain::QuestionId::from("q_reply_1"),
    )
    .expect("get q")
    .expect("exists");
    assert_eq!(
        still_pending.status,
        runlark_domain::QuestionStatus::Pending
    );
    assert_eq!(
        SessionStore::get_run(&conn, &rid)
            .expect("get run")
            .expect("exists")
            .status,
        RunStatus::WaitingForApproval
    );

    // Multi-part content is joined cleanly when answering
    let valid_reply = DshWireEvent {
        event_type: "user/message".to_string(),
        seq: 3,
        time: 2200,
        data: json!({
            "source": { "kind": "user-question-reply", "callId": "q_reply_1" },
            "content": [
                { "type": "text", "text": "Approve" },
                { "type": "text", "text": "with tests" }
            ]
        }),
    };
    process_dsh_wire_event(
        &mut conn,
        &state,
        &sid,
        Some("run_reply"),
        &valid_reply,
        Timestamp(2200),
    )
    .expect("valid reply");

    let answered = runlark_local::QuestionStore::get_question(
        &conn,
        &runlark_domain::QuestionId::from("q_reply_1"),
    )
    .expect("get q")
    .expect("exists");
    assert_eq!(answered.status, runlark_domain::QuestionStatus::Answered);
    assert_eq!(answered.answer.as_deref(), Some("Approve\nwith tests"));
}
