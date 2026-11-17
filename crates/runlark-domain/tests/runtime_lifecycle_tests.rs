use runlark_domain::{
    AnswerQuestionCommand, CommandId, DomainEvent, QuestionId, Revision, Run, RunId, RunStatus,
    RuntimeEvent, SendPromptCommand, Session, SessionCommand, SessionId, SessionStatus,
    StopRunCommand, TaskId, Timestamp, TransitionError, TurnEndStatus, apply_runtime_event,
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

fn active_session_and_run(run_id: &str, status: RunStatus) -> (Session, Run) {
    let mut session = sample_session();
    session.status = SessionStatus::Active;
    session.active_run_id = Some(RunId::from(run_id));
    let run = Run {
        id: RunId::from(run_id),
        session_id: session.id.clone(),
        status,
        worktree_path: "/workspace".to_string(),
        revision: Revision(1),
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
        ..Run::default()
    };
    (session, run)
}

#[test]
fn test_waiting_for_approval_question_and_answer_lifecycle() {
    let (session, running_run) = active_session_and_run("run_q", RunStatus::Running);

    // 1. DSH asks a human question -> Running transitions to WaitingForApproval
    let ask_event = RuntimeEvent::UserQuestionAsked {
        session_id: session.id.clone(),
        run_id: running_run.id.clone(),
        question_id: QuestionId::from("q_1"),
        prompt: "Deploy to staging?".to_string(),
        options: vec!["Approve".to_string(), "Reject".to_string()],
    };
    let asked =
        apply_runtime_event(&session, &running_run, &ask_event, Timestamp(2000)).expect("ask");
    assert_eq!(asked.new_session.current_revision, Revision(2));
    let waiting_run = asked.new_run.expect("run present");
    assert_eq!(waiting_run.status, RunStatus::WaitingForApproval);
    assert_eq!(waiting_run.revision, Revision(2));
    assert_eq!(
        asked.events,
        vec![DomainEvent::QuestionAsked {
            question_id: QuestionId::from("q_1"),
            session_id: session.id.clone(),
            run_id: waiting_run.id.clone(),
            prompt: "Deploy to staging?".to_string(),
            options: vec!["Approve".to_string(), "Reject".to_string()],
            timestamp: Timestamp(2000),
        }]
    );

    // 2. SendPrompt while WaitingForApproval must be rejected (no silent overwrite!)
    let send_while_waiting = SessionCommand::SendPrompt(SendPromptCommand {
        command_id: CommandId::from("cmd_prompt_blocked"),
        session_id: session.id.clone(),
        run_id: waiting_run.id.clone(),
        prompt: "Bypass approval".to_string(),
        expected_revision: asked.new_session.current_revision,
        created_at: Timestamp(2100),
    });
    assert_eq!(
        transition(
            &asked.new_session,
            Some(&waiting_run),
            &send_while_waiting,
            Timestamp(2100)
        )
        .unwrap_err(),
        TransitionError::RunWaitingForApproval
    );

    // 3. AnswerQuestion transitions WaitingForApproval -> Running and emits QuestionAnswered
    let answer_cmd = SessionCommand::AnswerQuestion(AnswerQuestionCommand {
        command_id: CommandId::from("cmd_answer_1"),
        session_id: session.id.clone(),
        run_id: waiting_run.id.clone(),
        question_id: QuestionId::from("q_1"),
        answer: "Approve".to_string(),
        expected_revision: asked.new_session.current_revision,
        created_at: Timestamp(2200),
    });
    let answered = transition(
        &asked.new_session,
        Some(&waiting_run),
        &answer_cmd,
        Timestamp(2200),
    )
    .expect("answer should succeed");
    assert_eq!(answered.new_session.current_revision, Revision(3));
    let resumed_run = answered.new_run.expect("run present");
    assert_eq!(resumed_run.status, RunStatus::Running);
    assert_eq!(resumed_run.revision, Revision(3));
    assert_eq!(
        answered.events,
        vec![DomainEvent::QuestionAnswered {
            question_id: QuestionId::from("q_1"),
            session_id: session.id.clone(),
            run_id: resumed_run.id.clone(),
            command_id: CommandId::from("cmd_answer_1"),
            answer: "Approve".to_string(),
            timestamp: Timestamp(2200),
        }]
    );

    // 4. Answering again while already Running fails with RunNotWaitingForApproval
    let duplicate_answer = SessionCommand::AnswerQuestion(AnswerQuestionCommand {
        command_id: CommandId::from("cmd_answer_2"),
        session_id: session.id.clone(),
        run_id: resumed_run.id.clone(),
        question_id: QuestionId::from("q_1"),
        answer: "Approve".to_string(),
        expected_revision: answered.new_session.current_revision,
        created_at: Timestamp(2300),
    });
    assert_eq!(
        transition(
            &answered.new_session,
            Some(&resumed_run),
            &duplicate_answer,
            Timestamp(2300)
        )
        .unwrap_err(),
        TransitionError::RunNotWaitingForApproval
    );
}

#[test]
fn test_stop_while_waiting_for_approval_reconciles_on_turn_end() {
    let mut session = sample_session();
    session.status = SessionStatus::Active;
    session.current_revision = Revision(2);
    session.active_run_id = Some(RunId::from("run_q_stop"));

    let waiting_run = Run {
        id: RunId::from("run_q_stop"),
        session_id: session.id.clone(),
        status: RunStatus::WaitingForApproval,
        worktree_path: "/workspace".to_string(),
        revision: Revision(2),
        created_at: Timestamp(1000),
        updated_at: Timestamp(2000),
        ..Run::default()
    };

    // StopRun while WaitingForApproval transitions cleanly to CancelRequested
    let stop_while_waiting = SessionCommand::StopRun(StopRunCommand {
        command_id: CommandId::from("cmd_stop_waiting"),
        session_id: session.id.clone(),
        run_id: waiting_run.id.clone(),
        reason: Some("User cancelled approval".to_string()),
        expected_revision: session.current_revision,
        created_at: Timestamp(2400),
    });
    let cancelling = transition(
        &session,
        Some(&waiting_run),
        &stop_while_waiting,
        Timestamp(2400),
    )
    .expect("stop while waiting should succeed");
    let cancelling_run = cancelling.new_run.expect("run present");
    assert_eq!(cancelling_run.status, RunStatus::CancelRequested);

    // turn/end after CancelRequested transitions cleanly to Stopped
    let turn_end_stopped = RuntimeEvent::TurnEnded {
        session_id: session.id,
        run_id: cancelling_run.id.clone(),
        outcome: TurnEndStatus::Completed,
    };
    let stopped = apply_runtime_event(
        &cancelling.new_session,
        &cancelling_run,
        &turn_end_stopped,
        Timestamp(2500),
    )
    .expect("turn/end should reconcile CancelRequested to Stopped");
    assert_eq!(stopped.new_session.active_run_id, None);
    let final_run = stopped.new_run.expect("run present");
    assert_eq!(final_run.status, RunStatus::Stopped);
    assert_eq!(
        stopped.events,
        vec![DomainEvent::RunStopped {
            run_id: final_run.id,
            timestamp: Timestamp(2500),
        }]
    );
}

#[test]
fn test_runtime_turn_start_transitions() {
    let mut session = sample_session();
    session.status = SessionStatus::Active;
    session.active_run_id = Some(RunId::from("run_rt"));

    let queued_run = Run {
        id: RunId::from("run_rt"),
        session_id: session.id.clone(),
        status: RunStatus::Queued,
        worktree_path: "/workspace".to_string(),
        revision: Revision(1),
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
        ..Run::default()
    };

    // Queued -> Running on TurnStarted
    let started = apply_runtime_event(
        &session,
        &queued_run,
        &RuntimeEvent::TurnStarted {
            session_id: session.id.clone(),
            run_id: queued_run.id.clone(),
        },
        Timestamp(1100),
    )
    .expect("turn/start from queued");
    let running_run = started.new_run.expect("run");
    assert_eq!(running_run.status, RunStatus::Running);
    assert_eq!(started.new_session.current_revision, Revision(2));

    // Redundant TurnStarted while already Running is idempotent on revision
    let redundant_start = apply_runtime_event(
        &started.new_session,
        &running_run,
        &RuntimeEvent::TurnStarted {
            session_id: session.id.clone(),
            run_id: running_run.id.clone(),
        },
        Timestamp(1150),
    )
    .expect("idempotent turn/start");
    assert_eq!(redundant_start.new_session.current_revision, Revision(2));

    // Historical TurnStarted replay while Interrupted must NOT clear Interrupted
    let mut interrupted_run = running_run;
    interrupted_run.status = RunStatus::Interrupted;
    let interrupted_after_replay = apply_runtime_event(
        &started.new_session,
        &interrupted_run,
        &RuntimeEvent::TurnStarted {
            session_id: session.id,
            run_id: interrupted_run.id.clone(),
        },
        Timestamp(1175),
    )
    .expect("turn/start replay on interrupted preserves interrupted");
    assert_eq!(
        interrupted_after_replay.new_run.expect("run").status,
        RunStatus::Interrupted
    );
}

#[test]
fn test_runtime_turn_end_outcomes() {
    let mut session = sample_session();
    session.status = SessionStatus::Active;
    session.active_run_id = Some(RunId::from("run_rt"));
    session.current_revision = Revision(2);

    let running_run = Run {
        id: RunId::from("run_rt"),
        session_id: session.id.clone(),
        status: RunStatus::Running,
        worktree_path: "/workspace".to_string(),
        revision: Revision(2),
        created_at: Timestamp(1000),
        updated_at: Timestamp(1100),
        ..Run::default()
    };

    // Running -> Completed on TurnEnded(Completed)
    let completed = apply_runtime_event(
        &session,
        &running_run,
        &RuntimeEvent::TurnEnded {
            session_id: session.id.clone(),
            run_id: running_run.id.clone(),
            outcome: TurnEndStatus::Completed,
        },
        Timestamp(1200),
    )
    .expect("turn/end completed");
    let completed_run = completed.new_run.expect("run");
    assert_eq!(completed_run.status, RunStatus::Completed);
    assert_eq!(completed.new_session.active_run_id, None);
    assert_eq!(
        completed.events,
        vec![DomainEvent::RunCompleted {
            run_id: completed_run.id.clone(),
            timestamp: Timestamp(1200),
        }]
    );

    // Further TurnEnded on already terminal run is rejected
    assert_eq!(
        apply_runtime_event(
            &completed.new_session,
            &completed_run,
            &RuntimeEvent::TurnEnded {
                session_id: session.id.clone(),
                run_id: completed_run.id.clone(),
                outcome: TurnEndStatus::Stopped,
            },
            Timestamp(1250),
        )
        .unwrap_err(),
        TransitionError::RunAlreadyTerminated
    );

    // Running -> Failed on TurnEnded(Failed)
    let failed = apply_runtime_event(
        &session,
        &running_run,
        &RuntimeEvent::TurnEnded {
            session_id: session.id.clone(),
            run_id: running_run.id.clone(),
            outcome: TurnEndStatus::Failed {
                error: "sandbox crash".to_string(),
            },
        },
        Timestamp(1300),
    )
    .expect("turn/end failed");
    let failed_run = failed.new_run.expect("run");
    assert_eq!(failed_run.status, RunStatus::Failed);
    assert_eq!(failed.new_session.active_run_id, None);
    assert_eq!(
        failed.events,
        vec![DomainEvent::RunFailed {
            run_id: failed_run.id,
            error: "sandbox crash".to_string(),
            timestamp: Timestamp(1300),
        }]
    );
}
