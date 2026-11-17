use runlark_domain::{
    CommandId, EventSeq, Revision, Run, RunId, RunStatus, SendPromptCommand, Session,
    SessionCommand, SessionId, SessionStatus, TaskId, Timestamp,
};
use runlark_local::{
    CommandStore, EventStore, PersistOutcome, SessionStore, StorageError, initialize_schema,
    open_in_memory,
};

#[test]
fn test_persist_intent_idempotency_and_conflict() {
    let mut conn = open_in_memory().expect("open DB");
    initialize_schema(&conn).expect("init schema");

    let session = Session {
        id: SessionId::from("sess_1"),
        task_id: TaskId::from("task_1"),
        cwd: "/workspace".to_string(),
        status: SessionStatus::Active,
        current_revision: Revision(1),
        active_run_id: Some(RunId::from("run_1")),
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
    };
    SessionStore::create_session(&conn, &session).expect("create session");

    let run = Run {
        id: RunId::from("run_1"),
        session_id: session.id.clone(),
        status: RunStatus::Running,
        worktree_path: "/workspace".to_string(),
        revision: Revision(1),
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
    };
    SessionStore::save_run(&conn, &run).expect("save run");

    let cmd_payload = SendPromptCommand {
        command_id: CommandId::from("cmd_test_1"),
        session_id: session.id.clone(),
        run_id: run.id.clone(),
        prompt: "Run tests".to_string(),
        expected_revision: Revision(1),
        created_at: Timestamp(1050),
    };
    let cmd = SessionCommand::SendPrompt(cmd_payload.clone());

    // 1. Fresh insert -> Inserted
    let outcome1 =
        CommandStore::persist_intent(&mut conn, &cmd, Timestamp(1050)).expect("persist intent");
    assert_eq!(outcome1, PersistOutcome::Inserted);

    // 2. Same command ID with identical payload -> AlreadyExists
    let outcome2 = CommandStore::persist_intent(&mut conn, &cmd, Timestamp(1060))
        .expect("persist identical intent");
    assert_eq!(
        outcome2,
        PersistOutcome::AlreadyExists {
            status: "pending".to_string()
        }
    );

    // 3. Same command ID with modified payload -> Conflict!
    let mut modified_payload = cmd_payload;
    modified_payload.prompt = "Different prompt".to_string();
    let conflict_cmd = SessionCommand::SendPrompt(modified_payload);

    let err = CommandStore::persist_intent(&mut conn, &conflict_cmd, Timestamp(1070))
        .expect_err("must reject conflict");

    match err {
        StorageError::CommandPayloadConflict(c_id) => {
            assert_eq!(c_id, CommandId::from("cmd_test_1"));
        }
        other => panic!("expected CommandPayloadConflict, got {other:?}"),
    }

    // 4. Marking dispatched and acked
    CommandStore::mark_dispatched(&conn, &CommandId::from("cmd_test_1"), Timestamp(1100))
        .expect("mark dispatched");
    let fetched = CommandStore::get_command(&conn, &CommandId::from("cmd_test_1"))
        .expect("fetch cmd")
        .expect("found");
    assert_eq!(fetched.status, "dispatched");
    assert_eq!(fetched.dispatch_attempts, 1);

    CommandStore::mark_acked(&conn, &CommandId::from("cmd_test_1"), Timestamp(1150))
        .expect("mark acked");
    let fetched_acked = CommandStore::get_command(&conn, &CommandId::from("cmd_test_1"))
        .expect("fetch cmd")
        .expect("found");
    assert_eq!(fetched_acked.status, "acked");

    // 5. Missing command_id returns InvalidState instead of silent Ok(())
    let missing_id = CommandId::from("cmd_missing");
    assert!(matches!(
        CommandStore::mark_dispatched(&conn, &missing_id, Timestamp(1200)),
        Err(StorageError::InvalidState(_))
    ));
    assert!(matches!(
        CommandStore::mark_acked(&conn, &missing_id, Timestamp(1200)),
        Err(StorageError::InvalidState(_))
    ));
    assert!(matches!(
        CommandStore::mark_failed(&conn, &missing_id, "err", Timestamp(1200)),
        Err(StorageError::InvalidState(_))
    ));

    // 6. Values exceeding SQLite i64::MAX are rejected instead of silently clamped
    let overflow_cmd = SessionCommand::SendPrompt(SendPromptCommand {
        command_id: CommandId::from("cmd_overflow"),
        session_id: session.id,
        run_id: run.id,
        prompt: "Overflow rev".to_string(),
        expected_revision: Revision(u64::MAX),
        created_at: Timestamp(1250),
    });
    assert!(matches!(
        CommandStore::persist_intent(&mut conn, &overflow_cmd, Timestamp(1250)),
        Err(StorageError::InvalidState(_))
    ));
}

#[test]
fn test_event_store_append_and_catchup() {
    let conn = open_in_memory().expect("open DB");
    initialize_schema(&conn).expect("init schema");

    let session = Session {
        id: SessionId::from("sess_evt"),
        task_id: TaskId::from("task_1"),
        cwd: "/workspace".to_string(),
        status: SessionStatus::Active,
        current_revision: Revision(1),
        active_run_id: Some(RunId::from("run_evt")),
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
    };
    SessionStore::create_session(&conn, &session).expect("create session");

    let run = Run {
        id: RunId::from("run_evt"),
        session_id: session.id.clone(),
        status: RunStatus::Running,
        worktree_path: "/workspace".to_string(),
        revision: Revision(1),
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
    };
    SessionStore::save_run(&conn, &run).expect("save run");

    // Append 3 events
    EventStore::append_event(
        &conn,
        &run.id,
        EventSeq(1),
        "PromptEnqueued",
        "{}",
        Timestamp(1010),
    )
    .expect("append 1");

    EventStore::append_event(
        &conn,
        &run.id,
        EventSeq(2),
        "StepStarted",
        "{}",
        Timestamp(1020),
    )
    .expect("append 2");

    EventStore::append_event(
        &conn,
        &run.id,
        EventSeq(3),
        "StepFinished",
        "{}",
        Timestamp(1030),
    )
    .expect("append 3");

    // All events
    let all = EventStore::get_events(&conn, &run.id, None).expect("get all");
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].seq, EventSeq(1));
    assert_eq!(all[2].seq, EventSeq(3));

    // Stream catch up: afterSeq = 1 returns seq 2 and 3
    let catchup = EventStore::get_events(&conn, &run.id, Some(EventSeq(1))).expect("get catchup");
    assert_eq!(catchup.len(), 2);
    assert_eq!(catchup[0].seq, EventSeq(2));
    assert_eq!(catchup[1].seq, EventSeq(3));
}

#[test]
fn test_event_store_includes_seq_zero_on_full_replay() {
    let conn = open_in_memory().expect("open DB");
    initialize_schema(&conn).expect("init schema");

    let session = Session {
        id: SessionId::from("sess_evt_zero"),
        task_id: TaskId::from("task_0"),
        cwd: "/workspace".to_string(),
        status: SessionStatus::Active,
        current_revision: Revision(1),
        active_run_id: Some(RunId::from("run_evt_zero")),
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
    };
    SessionStore::create_session(&conn, &session).expect("create session");

    let run = Run {
        id: RunId::from("run_evt_zero"),
        session_id: session.id.clone(),
        status: RunStatus::Running,
        worktree_path: "/workspace".to_string(),
        revision: Revision(1),
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
    };
    SessionStore::save_run(&conn, &run).expect("save run");

    // Append event with seq 0 (initial turn event) and seq 1
    EventStore::append_event(
        &conn,
        &run.id,
        EventSeq(0),
        "RunStarted",
        "{}",
        Timestamp(1000),
    )
    .expect("append seq 0");

    EventStore::append_event(
        &conn,
        &run.id,
        EventSeq(1),
        "PromptEnqueued",
        "{}",
        Timestamp(1010),
    )
    .expect("append seq 1");

    // Replay with after_seq = None MUST include seq 0
    let all = EventStore::get_events(&conn, &run.id, None).expect("get all events");
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].seq, EventSeq(0));
    assert_eq!(all[0].event_type, "RunStarted");
    assert_eq!(all[1].seq, EventSeq(1));
    assert_eq!(all[1].event_type, "PromptEnqueued");

    // Catchup with after_seq = Some(0) returns only events > 0
    let after_zero =
        EventStore::get_events(&conn, &run.id, Some(EventSeq(0))).expect("get after seq 0");
    assert_eq!(after_zero.len(), 1);
    assert_eq!(after_zero[0].seq, EventSeq(1));

    // Catchup with after_seq = Some(1) returns empty vec
    let after_one =
        EventStore::get_events(&conn, &run.id, Some(EventSeq(1))).expect("get after seq 1");
    assert_eq!(after_one.len(), 0);
}
