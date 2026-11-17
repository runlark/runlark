use runlark_domain::{
    AnswerQuestionCommand, CommandId, QuestionId, QuestionStatus, Revision, Run, RunId, RunStatus,
    SendPromptCommand, Session, SessionCommand, SessionId, SessionStatus, TaskId, Timestamp,
    TurnEndStatus,
};
use runlark_local::{
    CommandApplyOutcome, CommandStore, EventStore, LocalStore, QuestionStore, ReconciliationEngine,
    SessionStore, initialize_schema, open_sqlite,
};
use tempfile::NamedTempFile;

#[test]
fn test_restart_recovery_reconciles_inflight_runs() {
    let temp_db = NamedTempFile::new().expect("create temp db file");
    let db_path = temp_db.path().to_path_buf();

    // 1. Process 1: Start, create running session & run, insert pending command, then "crash"
    {
        let mut conn = open_sqlite(&db_path).expect("open db");
        initialize_schema(&conn).expect("init schema");

        let session = Session {
            id: SessionId::from("sess_crash"),
            task_id: TaskId::from("task_crash"),
            cwd: "/workspace".to_string(),
            status: SessionStatus::Active,
            current_revision: Revision(1),
            active_run_id: Some(RunId::from("run_crash")),
            created_at: Timestamp(1000),
            updated_at: Timestamp(1000),
        };
        SessionStore::create_session(&conn, &session).expect("create session");

        let run = Run {
            id: RunId::from("run_crash"),
            session_id: session.id.clone(),
            status: RunStatus::Running,
            worktree_path: "/workspace".to_string(),
            revision: Revision(1),
            created_at: Timestamp(1000),
            updated_at: Timestamp(1000),
        };
        SessionStore::save_run(&conn, &run).expect("save run");

        let cmd = SessionCommand::SendPrompt(SendPromptCommand {
            command_id: CommandId::from("cmd_unacked"),
            session_id: session.id.clone(),
            run_id: run.id.clone(),
            prompt: "Test crash recovery".to_string(),
            expected_revision: Revision(1),
            created_at: Timestamp(1050),
        });
        CommandStore::persist_intent(&mut conn, &cmd, Timestamp(1050)).expect("persist cmd");

        // Connection drops here (simulating abrupt daemon crash / termination)
    }

    // 2. Process 2: Daemon restarts with the same SQLite database file
    {
        let mut conn = open_sqlite(&db_path).expect("reopen db after crash");

        // Scan inflight before reconciliation
        let before = ReconciliationEngine::scan_inflight(&conn).expect("scan inflight");
        assert_eq!(before.active_runs.len(), 1);
        assert_eq!(before.active_runs[0].0, RunId::from("run_crash"));
        assert_eq!(before.active_runs[0].2, RunStatus::Running);
        assert_eq!(before.unacked_commands.len(), 1);
        assert_eq!(before.unacked_commands[0].0, CommandId::from("cmd_unacked"));
        assert_eq!(before.unacked_commands[0].3, "pending");

        // Execute startup reconciliation
        let reconciled = ReconciliationEngine::reconcile_on_startup(&mut conn, Timestamp(2000))
            .expect("reconciliation");
        assert_eq!(reconciled.active_runs.len(), 1);

        // Verify that the run status was recovered to Interrupted
        let run_after = SessionStore::get_run(&conn, &RunId::from("run_crash"))
            .expect("fetch run")
            .expect("run exists");
        assert_eq!(run_after.status, RunStatus::Interrupted);
        assert_eq!(run_after.updated_at, Timestamp(2000));

        // After reconciliation, active_runs query returns 0
        let after = ReconciliationEngine::scan_inflight(&conn).expect("scan inflight after");
        assert_eq!(after.active_runs.len(), 0);
        // The unacked command is preserved in durable outbox for retry/reconciliation
        assert_eq!(after.unacked_commands.len(), 1);
    }
}

#[test]
fn test_restart_recovery_reconciles_cancelling_and_queued_runs() {
    let temp_db = NamedTempFile::new().expect("create temp db file");
    let db_path = temp_db.path().to_path_buf();

    // 1. Seed DB with runs in CancelRequested and Queued states
    {
        let conn = open_sqlite(&db_path).expect("open db");
        initialize_schema(&conn).expect("init schema");

        let session1 = Session {
            id: SessionId::from("sess_cancelling"),
            task_id: TaskId::from("task_cancelling"),
            cwd: "/workspace".to_string(),
            status: SessionStatus::Active,
            current_revision: Revision(1),
            active_run_id: Some(RunId::from("run_cancelling")),
            created_at: Timestamp(1000),
            updated_at: Timestamp(1000),
        };
        SessionStore::create_session(&conn, &session1).expect("create session 1");

        let run_cancelling = Run {
            id: RunId::from("run_cancelling"),
            session_id: session1.id.clone(),
            status: RunStatus::CancelRequested,
            worktree_path: "/workspace/1".to_string(),
            revision: Revision(1),
            created_at: Timestamp(1000),
            updated_at: Timestamp(1000),
        };
        SessionStore::save_run(&conn, &run_cancelling).expect("save run cancelling");

        let session2 = Session {
            id: SessionId::from("sess_queued"),
            task_id: TaskId::from("task_queued"),
            cwd: "/workspace".to_string(),
            status: SessionStatus::Active,
            current_revision: Revision(1),
            active_run_id: Some(RunId::from("run_queued")),
            created_at: Timestamp(1100),
            updated_at: Timestamp(1100),
        };
        SessionStore::create_session(&conn, &session2).expect("create session 2");

        let run_queued = Run {
            id: RunId::from("run_queued"),
            session_id: session2.id.clone(),
            status: RunStatus::Queued,
            worktree_path: "/workspace/2".to_string(),
            revision: Revision(1),
            created_at: Timestamp(1100),
            updated_at: Timestamp(1100),
        };
        SessionStore::save_run(&conn, &run_queued).expect("save run queued");
    }

    // 2. Restart and reconcile
    {
        let mut conn = open_sqlite(&db_path).expect("reopen db after crash");

        // Scan inflight before reconciliation should detect both CancelRequested and Queued runs
        let before = ReconciliationEngine::scan_inflight(&conn).expect("scan inflight");
        assert_eq!(before.active_runs.len(), 2);

        // Execute startup reconciliation
        let reconciled = ReconciliationEngine::reconcile_on_startup(&mut conn, Timestamp(3000))
            .expect("reconciliation");
        assert_eq!(reconciled.active_runs.len(), 2);

        // Verify that both runs transitioned to Interrupted
        let fetched_cancelling = SessionStore::get_run(&conn, &RunId::from("run_cancelling"))
            .expect("fetch cancelling run")
            .expect("cancelling run exists");
        assert_eq!(fetched_cancelling.status, RunStatus::Interrupted);
        assert_eq!(fetched_cancelling.updated_at, Timestamp(3000));

        let fetched_queued = SessionStore::get_run(&conn, &RunId::from("run_queued"))
            .expect("fetch queued run")
            .expect("queued run exists");
        assert_eq!(fetched_queued.status, RunStatus::Interrupted);
        assert_eq!(fetched_queued.updated_at, Timestamp(3000));

        // After reconciliation, active_runs query returns 0
        let after = ReconciliationEngine::scan_inflight(&conn).expect("scan inflight after");
        assert_eq!(after.active_runs.len(), 0);

        // Terminal runs must never be overwritten by mark_interrupted
        let completed_run = Run {
            id: RunId::from("run_done"),
            session_id: SessionId::from("sess_queued"),
            status: RunStatus::Completed,
            worktree_path: "/workspace/done".to_string(),
            revision: Revision(1),
            created_at: Timestamp(1100),
            updated_at: Timestamp(1200),
        };
        SessionStore::save_run(&conn, &completed_run).expect("save completed run");
        ReconciliationEngine::mark_interrupted(&conn, &completed_run.id, Timestamp(4000))
            .expect("mark_interrupted should not error on terminal run");
        let fetched_done = SessionStore::get_run(&conn, &completed_run.id)
            .expect("fetch completed run")
            .expect("completed run exists");
        assert_eq!(fetched_done.status, RunStatus::Completed);
        assert_eq!(fetched_done.updated_at, Timestamp(1200));
    }
}

fn seed_waiting_and_active_runs(db_path: &std::path::Path) {
    let mut conn = open_sqlite(db_path).expect("open db");
    initialize_schema(&conn).expect("init schema");

    let sess_wait = Session {
        id: SessionId::from("sess_wait"),
        task_id: TaskId::from("task_wait"),
        cwd: "/workspace/wait".to_string(),
        status: SessionStatus::Created,
        current_revision: Revision(1),
        active_run_id: None,
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
    };
    SessionStore::create_session(&conn, &sess_wait).expect("create waiting session");

    let prompt_cmd = SessionCommand::SendPrompt(SendPromptCommand {
        command_id: CommandId::from("cmd_wait_start"),
        session_id: sess_wait.id.clone(),
        run_id: RunId::from("run_wait"),
        prompt: "Prepare release".to_string(),
        expected_revision: Revision(1),
        created_at: Timestamp(1010),
    });
    LocalStore::apply_command(&mut conn, &prompt_cmd, Timestamp(1010)).expect("start run");

    LocalStore::record_user_question(
        &mut conn,
        &sess_wait.id,
        &RunId::from("run_wait"),
        &QuestionId::from("q_approve"),
        "Approve production deploy?",
        &["Approve".to_string(), "Abort".to_string()],
        Timestamp(1020),
    )
    .expect("record question");

    let sess_active = Session {
        id: SessionId::from("sess_active"),
        task_id: TaskId::from("task_active"),
        cwd: "/workspace/active".to_string(),
        status: SessionStatus::Active,
        current_revision: Revision(1),
        active_run_id: Some(RunId::from("run_active")),
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
    };
    SessionStore::create_session(&conn, &sess_active).expect("create active session");
    SessionStore::save_run(
        &conn,
        &Run {
            id: RunId::from("run_active"),
            session_id: sess_active.id,
            status: RunStatus::Running,
            worktree_path: "/workspace/active".to_string(),
            revision: Revision(1),
            created_at: Timestamp(1000),
            updated_at: Timestamp(1000),
        },
    )
    .expect("save active run");
}

#[test]
fn test_restart_recovery_preserves_waiting_for_approval_and_pending_questions() {
    let temp_db = NamedTempFile::new().expect("create temp db file");
    let db_path = temp_db.path().to_path_buf();
    seed_waiting_and_active_runs(&db_path);

    let mut conn = open_sqlite(&db_path).expect("reopen db");
    let inflight =
        ReconciliationEngine::reconcile_on_startup(&mut conn, Timestamp(2000)).expect("reconcile");

    assert_eq!(inflight.active_runs.len(), 1);
    assert_eq!(inflight.active_runs[0].0, RunId::from("run_active"));
    assert_eq!(
        inflight.waiting_runs,
        vec![(RunId::from("run_wait"), SessionId::from("sess_wait"))]
    );
    assert_eq!(inflight.pending_questions.len(), 1);
    assert_eq!(
        inflight.pending_questions[0].id,
        QuestionId::from("q_approve")
    );

    // Running run became Interrupted and recorded RunInterrupted event
    let active_after = SessionStore::get_run(&conn, &RunId::from("run_active"))
        .expect("get active run")
        .expect("exists");
    assert_eq!(active_after.status, RunStatus::Interrupted);
    let active_events =
        EventStore::get_events(&conn, &RunId::from("run_active"), None).expect("events");
    assert_eq!(active_events.len(), 1);
    assert_eq!(active_events[0].event_type, "RunInterrupted");

    // WaitingForApproval run and Pending question MUST survive restart intact
    let wait_run = SessionStore::get_run(&conn, &RunId::from("run_wait"))
        .expect("get wait run")
        .expect("exists");
    assert_eq!(wait_run.status, RunStatus::WaitingForApproval);
    let q_after = QuestionStore::get_question(&conn, &QuestionId::from("q_approve"))
        .expect("get question")
        .expect("exists");
    assert_eq!(q_after.status, QuestionStatus::Pending);

    // User answers the question after restart, then turn completes cleanly
    let sess_now = SessionStore::get_session(&conn, &SessionId::from("sess_wait"))
        .expect("get sess")
        .expect("exists");
    let answer_outcome = LocalStore::answer_question(
        &mut conn,
        AnswerQuestionCommand {
            command_id: CommandId::from("cmd_answer_after_restart"),
            session_id: sess_now.id.clone(),
            run_id: wait_run.id.clone(),
            question_id: QuestionId::from("q_approve"),
            answer: "Approve".to_string(),
            expected_revision: sess_now.current_revision,
            created_at: Timestamp(3000),
        },
        Timestamp(3000),
    )
    .expect("answer question after restart");
    assert!(matches!(
        answer_outcome,
        CommandApplyOutcome::Applied { .. }
    ));

    let resumed_run = SessionStore::get_run(&conn, &wait_run.id)
        .expect("get resumed run")
        .expect("exists");
    assert_eq!(resumed_run.status, RunStatus::Running);
    assert!(
        QuestionStore::list_pending(&conn)
            .expect("list pending")
            .is_empty()
    );

    let finished = LocalStore::record_turn_end(
        &mut conn,
        &sess_now.id,
        &wait_run.id,
        TurnEndStatus::Completed,
        Timestamp(4000),
    )
    .expect("complete turn");
    assert_eq!(finished.run.status, RunStatus::Completed);
    assert_eq!(finished.session.active_run_id, None);
}
