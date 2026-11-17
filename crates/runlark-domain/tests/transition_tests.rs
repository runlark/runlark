use runlark_domain::{
    ArchiveSessionCommand, ChangeCwdCommand, CommandId, DomainEvent, EventSeq,
    ResumeSessionCommand, Revision, Run, RunId, RunStatus, SendPromptCommand, Session,
    SessionCommand, SessionId, SessionStatus, StopRunCommand, TaskId, Timestamp, TransitionError,
    transition,
};

fn sample_session() -> Session {
    Session {
        id: SessionId::from("sess_1"),
        task_id: TaskId::from("task_1"),
        cwd: "/workspace".to_string(),
        status: SessionStatus::Created,
        current_revision: Revision(1),
        active_run_id: None,
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
    }
}

#[test]
fn test_send_prompt_creates_and_starts_run() {
    let session = sample_session();
    let cmd = SessionCommand::SendPrompt(SendPromptCommand {
        command_id: CommandId::from("cmd_1"),
        session_id: session.id.clone(),
        run_id: RunId::from("run_1"),
        prompt: "Hello Runlark".to_string(),
        expected_revision: Revision(1),
        created_at: Timestamp(2000),
    });

    let outcome =
        transition(&session, None, &cmd, Timestamp(2000)).expect("transition should succeed");

    assert_eq!(outcome.new_session.status, SessionStatus::Active);
    assert_eq!(outcome.new_session.current_revision, Revision(2));
    assert_eq!(
        outcome.new_session.active_run_id,
        Some(RunId::from("run_1"))
    );

    let run = outcome.new_run.expect("run should be present");
    assert_eq!(run.id, RunId::from("run_1"));
    assert_eq!(run.status, RunStatus::Running);

    assert_eq!(outcome.events.len(), 1);
    match &outcome.events[0] {
        DomainEvent::PromptEnqueued { run_id, prompt, .. } => {
            assert_eq!(run_id, &RunId::from("run_1"));
            assert_eq!(prompt, "Hello Runlark");
        }
        _ => panic!("unexpected event"),
    }
}

#[test]
fn test_stop_run_transitions_to_cancel_requested_not_stopped() {
    let mut session = sample_session();
    session.status = SessionStatus::Active;
    session.active_run_id = Some(RunId::from("run_1"));

    let active_run = Run {
        id: RunId::from("run_1"),
        session_id: session.id.clone(),
        status: RunStatus::Running,
        worktree_path: "/workspace".to_string(),
        revision: Revision(1),
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
    };

    let cmd = SessionCommand::StopRun(StopRunCommand {
        command_id: CommandId::from("cmd_stop"),
        session_id: session.id.clone(),
        run_id: RunId::from("run_1"),
        reason: Some("User requested stop".to_string()),
        expected_revision: Revision(1),
        created_at: Timestamp(3000),
    });

    let outcome = transition(&session, Some(&active_run), &cmd, Timestamp(3000))
        .expect("transition should succeed");

    let run = outcome.new_run.expect("run should be present");
    // HARD INVARIANT: Cancel requested is NOT stopped!
    assert_eq!(run.status, RunStatus::CancelRequested);

    assert_eq!(outcome.events.len(), 1);
    match &outcome.events[0] {
        DomainEvent::RunCancelRequested { reason, .. } => {
            assert_eq!(reason.as_deref(), Some("User requested stop"));
        }
        other => panic!("expected RunCancelRequested, got {other:?}"),
    }
}

#[test]
fn test_revision_mismatch_fails() {
    let session = sample_session();
    let cmd = SessionCommand::SendPrompt(SendPromptCommand {
        command_id: CommandId::from("cmd_1"),
        session_id: session.id.clone(),
        run_id: RunId::from("run_1"),
        prompt: "Hello".to_string(),
        expected_revision: Revision(999), // Wrong revision
        created_at: Timestamp(2000),
    });

    let err = transition(&session, None, &cmd, Timestamp(2000)).unwrap_err();
    assert_eq!(
        err,
        TransitionError::RevisionMismatch {
            expected: Revision(999),
            actual: Revision(1),
        }
    );
}

#[test]
fn test_cwd_and_archive_guards() {
    let mut session = sample_session();
    let active_run = Run {
        id: RunId::from("run_1"),
        session_id: session.id.clone(),
        status: RunStatus::Running,
        worktree_path: "/workspace".to_string(),
        revision: Revision(1),
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
    };

    // ChangeCwd should fail while run is running
    let cwd_cmd = SessionCommand::ChangeCwd(ChangeCwdCommand {
        command_id: CommandId::from("cmd_cwd"),
        session_id: session.id.clone(),
        run_id: RunId::from("run_1"),
        new_cwd: "/new_path".to_string(),
        expected_revision: Revision(1),
        created_at: Timestamp(2000),
    });
    assert_eq!(
        transition(&session, Some(&active_run), &cwd_cmd, Timestamp(2000)).unwrap_err(),
        TransitionError::CwdConflictActiveRun
    );

    // ArchiveSession should fail while run is running
    let archive_cmd = SessionCommand::ArchiveSession(ArchiveSessionCommand {
        command_id: CommandId::from("cmd_arch"),
        session_id: session.id.clone(),
        run_id: RunId::from("run_1"),
        expected_revision: Revision(1),
        created_at: Timestamp(2000),
    });
    assert_eq!(
        transition(&session, Some(&active_run), &archive_cmd, Timestamp(2000)).unwrap_err(),
        TransitionError::ArchiveBlockedByActiveRun
    );

    // ChangeCwd and ArchiveSession must also fail while run is WaitingForApproval
    let mut approval_run = active_run;
    approval_run.status = RunStatus::WaitingForApproval;
    assert_eq!(
        transition(&session, Some(&approval_run), &cwd_cmd, Timestamp(2000)).unwrap_err(),
        TransitionError::CwdConflictActiveRun
    );
    assert_eq!(
        transition(&session, Some(&approval_run), &archive_cmd, Timestamp(2000)).unwrap_err(),
        TransitionError::ArchiveBlockedByActiveRun
    );

    // When no active run, ChangeCwd and Archive succeed
    let outcome =
        transition(&session, None, &cwd_cmd, Timestamp(2000)).expect("cwd should succeed");
    assert_eq!(outcome.new_session.cwd, "/new_path");

    session.current_revision = Revision(2);
    let archive_cmd2 = SessionCommand::ArchiveSession(ArchiveSessionCommand {
        command_id: CommandId::from("cmd_arch2"),
        session_id: session.id.clone(),
        run_id: RunId::from("run_none"),
        expected_revision: Revision(2),
        created_at: Timestamp(3000),
    });
    let arch_outcome =
        transition(&session, None, &archive_cmd2, Timestamp(3000)).expect("archive should succeed");
    assert_eq!(arch_outcome.new_session.status, SessionStatus::Archived);

    // Subsequent command on archived session must fail
    assert_eq!(
        transition(&arch_outcome.new_session, None, &cwd_cmd, Timestamp(4000)).unwrap_err(),
        TransitionError::SessionAlreadyArchived
    );
}

#[test]
fn test_resume_session_recovers_interrupted_run() {
    let session = sample_session();
    let interrupted_run = Run {
        id: RunId::from("run_1"),
        session_id: session.id.clone(),
        status: RunStatus::Interrupted,
        worktree_path: "/workspace".to_string(),
        revision: Revision(1),
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
    };

    let cmd = SessionCommand::ResumeSession(ResumeSessionCommand {
        command_id: CommandId::from("cmd_resume"),
        session_id: session.id.clone(),
        run_id: RunId::from("run_1"),
        after_seq: Some(EventSeq(5)),
        expected_revision: Revision(1),
        created_at: Timestamp(2000),
    });

    let outcome = transition(&session, Some(&interrupted_run), &cmd, Timestamp(2000))
        .expect("resume should succeed");

    assert_eq!(outcome.new_session.current_revision, Revision(2));
    let run = outcome.new_run.expect("run should be present");
    assert_eq!(run.status, RunStatus::Running);
    assert_eq!(outcome.events.len(), 1);
    match &outcome.events[0] {
        DomainEvent::SessionResumed {
            run_id, after_seq, ..
        } => {
            assert_eq!(run_id, &RunId::from("run_1"));
            assert_eq!(*after_seq, Some(EventSeq(5)));
        }
        other => panic!("expected SessionResumed, got {other:?}"),
    }

    // Resuming without an active run must not emit SessionResumed for a non-existent run
    let no_run_outcome =
        transition(&session, None, &cmd, Timestamp(2100)).expect("resume without run succeeds");
    assert!(no_run_outcome.new_run.is_none());
    assert!(no_run_outcome.events.is_empty());
}

#[test]
fn test_session_mismatch_and_terminal_run_guards() {
    let session = sample_session();
    let wrong_session_cmd = SessionCommand::ArchiveSession(ArchiveSessionCommand {
        command_id: CommandId::from("cmd_wrong_sess"),
        session_id: SessionId::from("sess_other"),
        run_id: RunId::from("run_none"),
        expected_revision: Revision(1),
        created_at: Timestamp(2000),
    });
    assert_eq!(
        transition(&session, None, &wrong_session_cmd, Timestamp(2000)).unwrap_err(),
        TransitionError::SessionMismatch {
            expected: SessionId::from("sess_1"),
            actual: SessionId::from("sess_other"),
        }
    );

    let terminal_run = Run {
        id: RunId::from("run_done"),
        session_id: session.id.clone(),
        status: RunStatus::Completed,
        worktree_path: "/workspace".to_string(),
        revision: Revision(1),
        created_at: Timestamp(1000),
        updated_at: Timestamp(1500),
    };
    let reuse_terminal_cmd = SessionCommand::SendPrompt(SendPromptCommand {
        command_id: CommandId::from("cmd_reuse"),
        session_id: session.id.clone(),
        run_id: RunId::from("run_done"),
        prompt: "Cannot revive".to_string(),
        expected_revision: Revision(1),
        created_at: Timestamp(2000),
    });
    assert_eq!(
        transition(
            &session,
            Some(&terminal_run),
            &reuse_terminal_cmd,
            Timestamp(2000)
        )
        .unwrap_err(),
        TransitionError::RunAlreadyTerminated
    );
}
