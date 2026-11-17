use runlark_domain::{
    AnswerQuestionCommand, ChangeCwdCommand, CommandId, EventSeq, QuestionId, QuestionStatus,
    Revision, RunId, RunStatus, SendPromptCommand, Session, SessionCommand, SessionId,
    SessionStatus, StopRunCommand, TaskId, Timestamp, TransitionError, TurnEndStatus,
};
use runlark_local::{
    CommandApplyOutcome, CommandStore, EventStore, LocalStore, QuestionStore, SessionStore,
    StorageError, initialize_schema, open_in_memory,
};

fn seed_session(conn: &rusqlite::Connection, id: &str) -> Session {
    let session = Session {
        id: SessionId::from(id),
        task_id: TaskId::from("task_1"),
        cwd: "/workspace".to_string(),
        status: SessionStatus::Created,
        current_revision: Revision(1),
        active_run_id: None,
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
    };
    SessionStore::create_session(conn, &session).expect("create session");
    session
}

#[test]
fn test_atomic_command_transition_rollback_on_errors() {
    let mut conn = open_in_memory().expect("open memory db");
    initialize_schema(&conn).expect("init schema");
    let session = seed_session(&conn, "sess_rollback");

    // 1. Wrong expected_revision rolls back everything: no command, no run, no events
    let bad_rev_cmd = SessionCommand::SendPrompt(SendPromptCommand {
        command_id: CommandId::from("cmd_bad_rev"),
        session_id: session.id,
        run_id: RunId::from("run_bad"),
        prompt: "Should rollback".to_string(),
        expected_revision: Revision(99),
        created_at: Timestamp(1100),
    });
    let err = LocalStore::apply_command(&mut conn, &bad_rev_cmd, Timestamp(1100)).unwrap_err();
    assert!(matches!(
        err,
        StorageError::Transition(TransitionError::RevisionMismatch { .. })
    ));
    assert_eq!(
        CommandStore::get_command(&conn, &CommandId::from("cmd_bad_rev")).expect("query"),
        None
    );
    assert_eq!(
        SessionStore::get_run(&conn, &RunId::from("run_bad")).expect("query"),
        None
    );
    assert!(
        EventStore::get_events(&conn, &RunId::from("run_bad"), None)
            .expect("query")
            .is_empty()
    );

    // 2. Missing session fails cleanly
    let missing_sess_cmd = SessionCommand::SendPrompt(SendPromptCommand {
        command_id: CommandId::from("cmd_no_sess"),
        session_id: SessionId::from("sess_missing"),
        run_id: RunId::from("run_missing"),
        prompt: "No session".to_string(),
        expected_revision: Revision(1),
        created_at: Timestamp(1150),
    });
    assert!(matches!(
        LocalStore::apply_command(&mut conn, &missing_sess_cmd, Timestamp(1150)),
        Err(StorageError::SessionNotFound(_))
    ));
}

#[test]
fn test_atomic_command_transition_idempotency_and_conflict() {
    let mut conn = open_in_memory().expect("open memory db");
    initialize_schema(&conn).expect("init schema");
    let session = seed_session(&conn, "sess_atom");

    // 1. Valid SendPrompt atomically updates session, run, command, and event at seq 0
    let prompt_cmd = SessionCommand::SendPrompt(SendPromptCommand {
        command_id: CommandId::from("cmd_1"),
        session_id: session.id.clone(),
        run_id: RunId::from("run_1"),
        prompt: "Implement feature".to_string(),
        expected_revision: Revision(1),
        created_at: Timestamp(1200),
    });
    let applied = LocalStore::apply_command(&mut conn, &prompt_cmd, Timestamp(1200))
        .expect("apply prompt command");
    match applied {
        CommandApplyOutcome::Applied {
            status,
            session: s,
            run: Some(r),
            events,
        } => {
            assert_eq!(status, "pending");
            assert_eq!(s.status, SessionStatus::Active);
            assert_eq!(s.current_revision, Revision(2));
            assert_eq!(s.active_run_id, Some(RunId::from("run_1")));
            assert_eq!(r.status, RunStatus::Running);
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].seq, EventSeq(0));
            assert_eq!(events[0].event_type, "PromptEnqueued");
        }
        other => panic!("expected Applied, got {other:?}"),
    }

    // 2. Idempotent retry returns AlreadyExists without re-applying or duplicating events
    let retry = LocalStore::apply_command(&mut conn, &prompt_cmd, Timestamp(1250))
        .expect("idempotent retry");
    match retry {
        CommandApplyOutcome::AlreadyExists {
            status,
            session: s,
            run: Some(r),
        } => {
            assert_eq!(status, "pending");
            assert_eq!(s.current_revision, Revision(2));
            assert_eq!(r.status, RunStatus::Running);
        }
        other => panic!("expected AlreadyExists, got {other:?}"),
    }
    assert_eq!(
        EventStore::get_events(&conn, &RunId::from("run_1"), None)
            .expect("events")
            .len(),
        1
    );

    // 3. Same command_id with different payload fails with CommandPayloadConflict
    let conflict_cmd = SessionCommand::SendPrompt(SendPromptCommand {
        command_id: CommandId::from("cmd_1"),
        session_id: session.id.clone(),
        run_id: RunId::from("run_1"),
        prompt: "Mutated prompt".to_string(),
        expected_revision: Revision(1),
        created_at: Timestamp(1200),
    });
    assert!(matches!(
        LocalStore::apply_command(&mut conn, &conflict_cmd, Timestamp(1260)),
        Err(StorageError::CommandPayloadConflict(_))
    ));

    // 4. ChangeCwd blocked by active run rolls back completely
    let cwd_cmd = SessionCommand::ChangeCwd(ChangeCwdCommand {
        command_id: CommandId::from("cmd_cwd_blocked"),
        session_id: session.id,
        run_id: RunId::from("run_1"),
        new_cwd: "/other".to_string(),
        expected_revision: Revision(2),
        created_at: Timestamp(1300),
    });
    assert!(matches!(
        LocalStore::apply_command(&mut conn, &cwd_cmd, Timestamp(1300)),
        Err(StorageError::Transition(
            TransitionError::CwdConflictActiveRun
        ))
    ));
    assert_eq!(
        CommandStore::get_command(&conn, &CommandId::from("cmd_cwd_blocked")).expect("query"),
        None
    );
}

#[test]
fn test_stop_and_turn_end_reconciliation_and_terminal_run_guard() {
    let mut conn = open_in_memory().expect("open memory db");
    initialize_schema(&conn).expect("init schema");
    let session = seed_session(&conn, "sess_stop");

    let start_cmd = SessionCommand::SendPrompt(SendPromptCommand {
        command_id: CommandId::from("cmd_start"),
        session_id: session.id.clone(),
        run_id: RunId::from("run_stop"),
        prompt: "Start work".to_string(),
        expected_revision: Revision(1),
        created_at: Timestamp(1000),
    });
    LocalStore::apply_command(&mut conn, &start_cmd, Timestamp(1000)).expect("start");

    // StopRun transitions to CancelRequested (NOT Stopped!)
    let stop_cmd = SessionCommand::StopRun(StopRunCommand {
        command_id: CommandId::from("cmd_stop"),
        session_id: session.id.clone(),
        run_id: RunId::from("run_stop"),
        reason: Some("user clicked stop".to_string()),
        expected_revision: Revision(2),
        created_at: Timestamp(1100),
    });
    let stop_res = LocalStore::apply_command(&mut conn, &stop_cmd, Timestamp(1100)).expect("stop");
    match stop_res {
        CommandApplyOutcome::Applied {
            run: Some(r),
            events,
            ..
        } => {
            assert_eq!(r.status, RunStatus::CancelRequested);
            assert_eq!(events[0].seq, EventSeq(1));
            assert_eq!(events[0].event_type, "RunCancelRequested");
        }
        other => panic!("unexpected outcome: {other:?}"),
    }

    // DSH turn/end reconciles CancelRequested -> Stopped and clears active_run_id
    let ended = LocalStore::record_turn_end(
        &mut conn,
        &session.id,
        &RunId::from("run_stop"),
        TurnEndStatus::Completed,
        Timestamp(1200),
    )
    .expect("record turn end");
    assert_eq!(ended.run.status, RunStatus::Stopped);
    assert_eq!(ended.session.active_run_id, None);
    assert_eq!(ended.session.current_revision, Revision(4));
    assert_eq!(ended.events[0].seq, EventSeq(2));
    assert_eq!(ended.events[0].event_type, "RunStopped");

    // Reusing terminated run_id in a subsequent SendPrompt must still fail with RunAlreadyTerminated
    let reuse_cmd = SessionCommand::SendPrompt(SendPromptCommand {
        command_id: CommandId::from("cmd_reuse"),
        session_id: session.id,
        run_id: RunId::from("run_stop"),
        prompt: "Cannot reuse".to_string(),
        expected_revision: Revision(4),
        created_at: Timestamp(1300),
    });
    assert!(matches!(
        LocalStore::apply_command(&mut conn, &reuse_cmd, Timestamp(1300)),
        Err(StorageError::Transition(
            TransitionError::RunAlreadyTerminated
        ))
    ));
}

#[test]
fn test_outbox_retry_clears_error_and_guards_acked() {
    let mut conn = open_in_memory().expect("open memory db");
    initialize_schema(&conn).expect("init schema");
    let session = seed_session(&conn, "sess_outbox");

    let cmd1 = SessionCommand::SendPrompt(SendPromptCommand {
        command_id: CommandId::from("cmd_out_1"),
        session_id: session.id.clone(),
        run_id: RunId::from("run_out_1"),
        prompt: "First".to_string(),
        expected_revision: Revision(1),
        created_at: Timestamp(1000),
    });
    LocalStore::apply_command(&mut conn, &cmd1, Timestamp(1000)).expect("apply cmd1");

    let cmd2 = SessionCommand::StopRun(StopRunCommand {
        command_id: CommandId::from("cmd_out_2"),
        session_id: session.id,
        run_id: RunId::from("run_out_1"),
        reason: None,
        expected_revision: Revision(2),
        created_at: Timestamp(1010),
    });
    LocalStore::apply_command(&mut conn, &cmd2, Timestamp(1010)).expect("apply cmd2");

    assert_eq!(
        CommandStore::list_pending_commands(&conn)
            .expect("pending")
            .len(),
        2
    );

    // cmd1 fails -> disappears from pending, stays in dispatchable with error_message
    CommandStore::mark_dispatched(&conn, &CommandId::from("cmd_out_1"), Timestamp(1020))
        .expect("dispatch 1");
    CommandStore::mark_failed(
        &conn,
        &CommandId::from("cmd_out_1"),
        "connection refused",
        Timestamp(1030),
    )
    .expect("fail 1");

    let pending = CommandStore::list_pending_commands(&conn).expect("pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].command_id, CommandId::from("cmd_out_2"));

    let dispatchable = CommandStore::list_dispatchable_commands(&conn).expect("dispatchable");
    assert_eq!(dispatchable.len(), 2);

    // Retry dispatch clears stale error_message
    CommandStore::mark_dispatched(&conn, &CommandId::from("cmd_out_1"), Timestamp(1040))
        .expect("retry dispatch");
    let retried = CommandStore::get_command(&conn, &CommandId::from("cmd_out_1"))
        .expect("get")
        .expect("exists");
    assert_eq!(retried.status, "dispatched");
    assert_eq!(retried.dispatch_attempts, 2);
    assert_eq!(retried.error_message, None);

    // Fail again then ack directly also clears error_message
    CommandStore::mark_failed(
        &conn,
        &CommandId::from("cmd_out_1"),
        "transient 503",
        Timestamp(1050),
    )
    .expect("fail again");
    CommandStore::mark_acked(&conn, &CommandId::from("cmd_out_1"), Timestamp(1060)).expect("ack");
    let acked = CommandStore::get_command(&conn, &CommandId::from("cmd_out_1"))
        .expect("get")
        .expect("exists");
    assert_eq!(acked.status, "acked");
    assert_eq!(acked.error_message, None);

    // Cannot revert an acked command back to failed or dispatched
    assert!(matches!(
        CommandStore::mark_failed(
            &conn,
            &CommandId::from("cmd_out_1"),
            "late error",
            Timestamp(1070)
        ),
        Err(StorageError::InvalidState(_))
    ));
    assert!(matches!(
        CommandStore::mark_dispatched(&conn, &CommandId::from("cmd_out_1"), Timestamp(1080)),
        Err(StorageError::InvalidState(_))
    ));
}

#[test]
fn test_question_store_lifecycle_and_turn_end_cancellation() {
    let mut conn = open_in_memory().expect("open memory db");
    initialize_schema(&conn).expect("init schema");
    let session = seed_session(&conn, "sess_q");

    let start = SessionCommand::SendPrompt(SendPromptCommand {
        command_id: CommandId::from("cmd_q_start"),
        session_id: session.id.clone(),
        run_id: RunId::from("run_q"),
        prompt: "Ask user".to_string(),
        expected_revision: Revision(1),
        created_at: Timestamp(1000),
    });
    LocalStore::apply_command(&mut conn, &start, Timestamp(1000)).expect("start");

    // 1. Record user question -> run WaitingForApproval, question Pending
    let asked = LocalStore::record_user_question(
        &mut conn,
        &session.id,
        &RunId::from("run_q"),
        &QuestionId::from("q_1"),
        "Choose environment",
        &["staging".to_string(), "prod".to_string()],
        Timestamp(1100),
    )
    .expect("ask question");
    assert_eq!(asked.run.status, RunStatus::WaitingForApproval);
    assert_eq!(asked.events[0].event_type, "QuestionAsked");

    let by_sess = QuestionStore::list_pending_for_session(&conn, &session.id).expect("by session");
    assert_eq!(by_sess.len(), 1);
    assert_eq!(by_sess[0].options, vec!["staging", "prod"]);

    let by_run = QuestionStore::list_pending_for_run(&conn, &RunId::from("run_q")).expect("by run");
    assert_eq!(by_run.len(), 1);

    // 2. Answer question -> run Running, question Answered
    LocalStore::answer_question(
        &mut conn,
        AnswerQuestionCommand {
            command_id: CommandId::from("cmd_q_ans"),
            session_id: session.id.clone(),
            run_id: RunId::from("run_q"),
            question_id: QuestionId::from("q_1"),
            answer: "staging".to_string(),
            expected_revision: asked.session.current_revision,
            created_at: Timestamp(1200),
        },
        Timestamp(1200),
    )
    .expect("answer question");

    let q1 = QuestionStore::get_question(&conn, &QuestionId::from("q_1"))
        .expect("get q1")
        .expect("exists");
    assert_eq!(q1.status, QuestionStatus::Answered);
    assert_eq!(q1.answer.as_deref(), Some("staging"));

    // 3. Ask a second question, then turn ends with Failed -> pending question is Cancelled
    LocalStore::record_user_question(
        &mut conn,
        &session.id,
        &RunId::from("run_q"),
        &QuestionId::from("q_2"),
        "Confirm rollback?",
        &["yes".to_string(), "no".to_string()],
        Timestamp(1300),
    )
    .expect("ask q2");

    let failed = LocalStore::record_turn_end(
        &mut conn,
        &session.id,
        &RunId::from("run_q"),
        TurnEndStatus::Failed {
            error: "timeout waiting for approval".to_string(),
        },
        Timestamp(1400),
    )
    .expect("turn fail");
    assert_eq!(failed.run.status, RunStatus::Failed);
    assert_eq!(failed.events[0].event_type, "RunFailed");

    let q2 = QuestionStore::get_question(&conn, &QuestionId::from("q_2"))
        .expect("get q2")
        .expect("exists");
    assert_eq!(q2.status, QuestionStatus::Cancelled);
    assert!(
        QuestionStore::list_pending(&conn)
            .expect("pending")
            .is_empty()
    );
}
