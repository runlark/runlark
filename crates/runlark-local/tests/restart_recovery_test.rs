use runlark_domain::{
    CommandId, Revision, Run, RunId, RunStatus, SendPromptCommand, Session, SessionCommand,
    SessionId, SessionStatus, TaskId, Timestamp,
};
use runlark_local::{
    CommandStore, ReconciliationEngine, SessionStore, initialize_schema, open_sqlite,
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
