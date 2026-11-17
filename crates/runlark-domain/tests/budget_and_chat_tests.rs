use runlark_domain::{
    Attempt, AttemptId, AttemptOutcome, BudgetBlockReason, BudgetPolicy, ChatParseError,
    CostSummary, DomainEvent, ParsedChatCommand, ProjectId, QuestionId, RepairScheduleOutcome, Run,
    RunId, RunStatus, TaskId, TaskKind, TaskStage, Timestamp, WorkflowStage, can_retry_repair,
    parse_chat_command, record_attempt_and_budget, schedule_repair,
};

fn make_attempt(no: u32, cost_micro_usd: Option<u64>, unknown_reason: Option<&str>) -> Attempt {
    Attempt {
        id: AttemptId::from(format!("att_{no}")),
        run_id: RunId::from("run_1"),
        attempt_no: no,
        workflow_stage: WorkflowStage::Implementation,
        started_at: Timestamp(1000),
        ended_at: Some(Timestamp(2000)),
        outcome: AttemptOutcome::Failed {
            error: "test failure".to_string(),
        },
        cost_micro_usd,
        unknown_cost_reason: unknown_reason.map(str::to_string),
        repair_reason: None,
    }
}

#[test]
fn test_cost_summary_preserves_unknown_attempts_never_zero() {
    let attempts = vec![
        make_attempt(1, Some(150_000), None),
        make_attempt(2, None, Some("provider omitted usage")),
    ];
    let summary = CostSummary::from_attempts(&attempts, 50_000);
    assert_eq!(summary.known_micro_usd, 150_000);
    assert_eq!(summary.held_micro_usd, 50_000);
    assert_eq!(summary.unknown_attempts, 1);
    assert!(!summary.is_complete());
    assert_eq!(summary.total_committed_and_held(), 200_000);

    let run = Run {
        id: RunId::from("run_1"),
        status: RunStatus::Running,
        workflow_stage: WorkflowStage::Verification,
        ..Run::default()
    };
    let outcome = record_attempt_and_budget(&run, &attempts[..1], &attempts[1], Timestamp(2500));
    assert_eq!(outcome.new_run.cost_summary.unknown_attempts, 1);
    assert_eq!(outcome.new_run.cost_summary.known_micro_usd, 150_000);
    assert!(
        outcome
            .events
            .iter()
            .any(|e| matches!(e, DomainEvent::UnknownCostRecorded { .. }))
    );
}

#[test]
fn test_can_retry_repair_and_schedule_repair_bounds() {
    let run = Run {
        id: RunId::from("run_1"),
        status: RunStatus::Running,
        workflow_stage: WorkflowStage::Verification,
        ..Run::default()
    };

    let policy = BudgetPolicy {
        max_attempts: 3,
        max_duration_ms: 60_000,
        max_spend_micro_usd: 1_000_000,
        allow_unknown_cost_repair: false,
    };

    // 1. Within budget -> allows repair and advances Verification -> Implementation
    let ok_attempts = vec![make_attempt(1, Some(200_000), None)];
    assert_eq!(
        can_retry_repair(&run, &ok_attempts, &policy, 10_000),
        Ok(())
    );
    let scheduled = schedule_repair(
        &run,
        &ok_attempts,
        &policy,
        10_000,
        "clippy failed",
        Timestamp(3000),
    )
    .expect("schedule repair");
    match scheduled {
        RepairScheduleOutcome::Scheduled { new_run, events } => {
            assert_eq!(new_run.workflow_stage, WorkflowStage::Implementation);
            assert_eq!(new_run.repair_attempts, 1);
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e, DomainEvent::RepairScheduled { .. }))
            );
        }
        RepairScheduleOutcome::Halted { .. } => panic!("expected Scheduled"),
    }

    // 2. Unknown cost blocks repair when allow_unknown_cost_repair is false
    let unknown_attempts = vec![make_attempt(1, None, Some("missing turn usage"))];
    assert_eq!(
        can_retry_repair(&run, &unknown_attempts, &policy, 10_000),
        Err(BudgetBlockReason::UnknownCostBlocked {
            unknown_attempts: 1
        })
    );
    let halted = schedule_repair(
        &run,
        &unknown_attempts,
        &policy,
        10_000,
        "retry",
        Timestamp(3100),
    )
    .expect("halt cleanly");
    match halted {
        RepairScheduleOutcome::Halted {
            new_run, reason, ..
        } => {
            assert_eq!(new_run.status, RunStatus::Failed);
            assert_eq!(
                reason,
                BudgetBlockReason::UnknownCostBlocked {
                    unknown_attempts: 1
                }
            );
        }
        RepairScheduleOutcome::Scheduled { .. } => panic!("expected Halted"),
    }

    // 3. Max attempts exceeded
    let three_attempts = vec![
        make_attempt(1, Some(10_000), None),
        make_attempt(2, Some(10_000), None),
        make_attempt(3, Some(10_000), None),
    ];
    assert_eq!(
        can_retry_repair(&run, &three_attempts, &policy, 10_000),
        Err(BudgetBlockReason::MaxAttemptsReached { used: 3, max: 3 })
    );

    // 4. Max duration exceeded
    assert_eq!(
        can_retry_repair(&run, &ok_attempts, &policy, 60_000),
        Err(BudgetBlockReason::MaxDurationExceeded {
            elapsed_ms: 60_000,
            max_ms: 60_000,
        })
    );

    // 5. Max spend exceeded (including held reservation)
    let mut run_with_hold = run;
    run_with_hold.cost_summary.held_micro_usd = 800_000;
    assert_eq!(
        can_retry_repair(&run_with_hold, &ok_attempts, &policy, 10_000),
        Err(BudgetBlockReason::MaxSpendExceeded {
            total_micro_usd: 1_000_000,
            max_micro_usd: 1_000_000,
        })
    );
}

#[test]
fn test_chat_command_parser_task_and_run_commands() {
    assert_eq!(
        parse_chat_command("/task new --quick \"Fix login race\"").unwrap(),
        ParsedChatCommand::CreateTask {
            title: "Fix login race".to_string(),
            kind: TaskKind::QuickFix,
        }
    );
    assert_eq!(
        parse_chat_command("/fix Broken border").unwrap(),
        ParsedChatCommand::CreateTask {
            title: "Broken border".to_string(),
            kind: TaskKind::QuickFix,
        }
    );
    assert_eq!(
        parse_chat_command("/task move task_1 in-progress").unwrap(),
        ParsedChatCommand::MoveTask {
            task_id: TaskId::from("task_1"),
            stage: TaskStage::InProgress,
        }
    );
    assert_eq!(
        parse_chat_command("/plan task_1 Add SQLite CAS store").unwrap(),
        ParsedChatCommand::UpdatePlan {
            task_id: TaskId::from("task_1"),
            plan_text: "Add SQLite CAS store".to_string(),
        }
    );
    assert_eq!(
        parse_chat_command("/plan approve task_1 looks solid").unwrap(),
        ParsedChatCommand::ApprovePlan {
            task_id: TaskId::from("task_1"),
            comment: Some("looks solid".to_string()),
        }
    );
    assert_eq!(
        parse_chat_command("/run task_1 --prompt \"Run tests\"").unwrap(),
        ParsedChatCommand::StartRun {
            task_id: TaskId::from("task_1"),
            prompt: Some("Run tests".to_string()),
        }
    );
    assert_eq!(
        parse_chat_command("/stop run_1 user abort").unwrap(),
        ParsedChatCommand::StopRun {
            run_id: RunId::from("run_1"),
            reason: Some("user abort".to_string()),
        }
    );
}

#[test]
fn test_chat_command_parser_review_and_query_commands() {
    assert_eq!(
        parse_chat_command("/answer q_1 Yes proceed").unwrap(),
        ParsedChatCommand::AnswerQuestion {
            question_id: QuestionId::from("q_1"),
            answer: "Yes proceed".to_string(),
        }
    );
    assert_eq!(
        parse_chat_command("/accept run_1 Ship it").unwrap(),
        ParsedChatCommand::Accept {
            target_id: "run_1".to_string(),
            comment: Some("Ship it".to_string()),
        }
    );
    assert_eq!(
        parse_chat_command("/reject run_1 Missing regression test").unwrap(),
        ParsedChatCommand::Reject {
            target_id: "run_1".to_string(),
            reason: "Missing regression test".to_string(),
        }
    );
    assert_eq!(
        parse_chat_command("/pr run_1 --title \"A1 domain\" --draft").unwrap(),
        ParsedChatCommand::CreatePr {
            run_id: RunId::from("run_1"),
            title: Some("A1 domain".to_string()),
            draft: true,
        }
    );
    assert_eq!(
        parse_chat_command("/status run_1").unwrap(),
        ParsedChatCommand::Status {
            target_id: Some("run_1".to_string()),
        }
    );
    assert_eq!(
        parse_chat_command("/board proj_1").unwrap(),
        ParsedChatCommand::Board {
            project_id: Some(ProjectId::from("proj_1")),
        }
    );
    assert_eq!(
        parse_chat_command("/budget run_1").unwrap(),
        ParsedChatCommand::Budget {
            target_id: Some("run_1".to_string()),
        }
    );
    assert_eq!(
        parse_chat_command("/help").unwrap(),
        ParsedChatCommand::Help
    );
    assert_eq!(
        parse_chat_command("Explain the failing test").unwrap(),
        ParsedChatCommand::Prompt {
            text: "Explain the failing test".to_string(),
        }
    );
}

#[test]
fn test_chat_command_parser_errors() {
    assert_eq!(parse_chat_command("   "), Err(ChatParseError::EmptyInput));
    assert_eq!(
        parse_chat_command("/unknown foo"),
        Err(ChatParseError::UnknownCommand("unknown".to_string()))
    );
    assert_eq!(
        parse_chat_command("/task \"unclosed"),
        Err(ChatParseError::UnterminatedQuote)
    );
    assert_eq!(
        parse_chat_command("/task move task_1 nope"),
        Err(ChatParseError::InvalidStage("nope".to_string()))
    );
    assert!(matches!(
        parse_chat_command("/reject run_1"),
        Err(ChatParseError::MissingArgument {
            command: "/reject",
            arg: "reason"
        })
    ));
}
