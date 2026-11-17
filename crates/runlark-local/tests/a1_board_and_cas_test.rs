use runlark_domain::{
    ActorRole, ArtifactDigest, ArtifactKind, AttachArtifactCommand, AttemptOutcome, BoardCommand,
    BudgetPolicy, CommandId, CreateProjectCommand, CreateTaskCommand, DecisionId, DecisionKind,
    MoveTaskStageCommand, PolicyDigest, ProjectId, RecordAttemptCommand, RecordDecisionCommand,
    Revision, Run, RunId, RunStatus, Session, SessionId, SessionStatus, TaskId, TaskKind,
    TaskStage, Timestamp, TransitionError, UpdateTaskPlanCommand, WorkflowStage,
};
use runlark_local::{
    ArtifactCasStore, AttemptStore, BoardCommandOutcome, BoardProjection, BoardStore,
    DecisionStore, ProjectStore, SessionStore, StorageError, TaskStore, initialize_schema,
    open_in_memory, open_sqlite,
};
use tempfile::TempDir;

fn seed_project_and_task(conn: &mut runlark_local::Connection) {
    let create_proj = BoardCommand::CreateProject(CreateProjectCommand {
        command_id: CommandId::from("cmd_proj_1"),
        project_id: ProjectId::from("proj_1"),
        name: "Runlark".to_string(),
        repo_root: "/repo".to_string(),
        default_branch: "main".to_string(),
        budget_policy: BudgetPolicy::default(),
        policy_digest: PolicyDigest::from("policy_v1"),
        created_at: Timestamp(1000),
    });
    assert!(matches!(
        BoardStore::apply_board_command(conn, &create_proj, Timestamp(1000)).unwrap(),
        BoardCommandOutcome::Applied { .. }
    ));

    // Idempotent replay returns AlreadyExists
    assert!(matches!(
        BoardStore::apply_board_command(conn, &create_proj, Timestamp(1001)).unwrap(),
        BoardCommandOutcome::AlreadyExists { .. }
    ));

    // Same command ID + different payload -> CommandPayloadConflict
    let mut conflict_proj = create_proj;
    if let BoardCommand::CreateProject(ref mut inner) = conflict_proj {
        inner.name = "Mutated".to_string();
    }
    assert!(matches!(
        BoardStore::apply_board_command(conn, &conflict_proj, Timestamp(1002)),
        Err(StorageError::CommandPayloadConflict(_))
    ));

    let create_task = BoardCommand::CreateTask(CreateTaskCommand {
        command_id: CommandId::from("cmd_task_1"),
        task_id: TaskId::from("task_1"),
        project_id: ProjectId::from("proj_1"),
        title: "Build A1 persistence".to_string(),
        description: "SQLite + CAS".to_string(),
        kind: TaskKind::Standard,
        base_ref: Some("main".to_string()),
        budget_override: None,
        created_at: Timestamp(1010),
    });
    BoardStore::apply_board_command(conn, &create_task, Timestamp(1010)).expect("create task");
}

fn seed_session_and_attempts(conn: &mut runlark_local::Connection) {
    let session = Session {
        id: SessionId::from("sess_1"),
        task_id: TaskId::from("task_1"),
        cwd: "/repo".to_string(),
        status: SessionStatus::Active,
        current_revision: Revision(1),
        active_run_id: Some(RunId::from("run_1")),
        created_at: Timestamp(1300),
        updated_at: Timestamp(1300),
    };
    SessionStore::create_session(conn, &session).expect("create session");
    let run = Run {
        id: RunId::from("run_1"),
        session_id: session.id,
        task_id: Some(TaskId::from("task_1")),
        project_id: Some(ProjectId::from("proj_1")),
        status: RunStatus::WaitingForApproval,
        workflow_stage: WorkflowStage::HumanAcceptance,
        worktree_path: "/repo/.runlark/worktrees/run_1".to_string(),
        base_sha: Some("base_sha_1".to_string()),
        head_sha: Some("head_sha_1".to_string()),
        plan_version: 2,
        policy_digest: Some(PolicyDigest::from("policy_v1")),
        revision: Revision(1),
        created_at: Timestamp(1300),
        updated_at: Timestamp(1300),
        ..Run::default()
    };
    SessionStore::save_run(conn, &run).expect("save run");

    let att1 = BoardCommand::RecordAttempt(RecordAttemptCommand {
        command_id: CommandId::from("cmd_att_1"),
        attempt_id: runlark_domain::AttemptId::from("att_1"),
        run_id: RunId::from("run_1"),
        workflow_stage: WorkflowStage::Implementation,
        outcome: AttemptOutcome::Failed {
            error: "clippy".to_string(),
        },
        cost_micro_usd: Some(25_000),
        unknown_cost_reason: None,
        repair_reason: None,
        expected_revision: Revision(1),
        created_at: Timestamp(1310),
    });
    BoardStore::apply_board_command(conn, &att1, Timestamp(1315)).expect("att1");

    let att2 = BoardCommand::RecordAttempt(RecordAttemptCommand {
        command_id: CommandId::from("cmd_att_2"),
        attempt_id: runlark_domain::AttemptId::from("att_2"),
        run_id: RunId::from("run_1"),
        workflow_stage: WorkflowStage::Implementation,
        outcome: AttemptOutcome::Succeeded,
        cost_micro_usd: None,
        unknown_cost_reason: Some("provider omitted cost".to_string()),
        repair_reason: Some("fix clippy".to_string()),
        expected_revision: Revision(2),
        created_at: Timestamp(1320),
    });
    BoardStore::apply_board_command(conn, &att2, Timestamp(1325)).expect("att2");
}

fn assert_decision_binding_and_accept(
    conn: &mut runlark_local::Connection,
    expected_revision: Revision,
) {
    let ev_digest = ArtifactDigest::parse("c".repeat(64)).expect("digest");
    let bad_dec = BoardCommand::RecordDecision(RecordDecisionCommand {
        command_id: CommandId::from("cmd_dec_bad"),
        decision_id: DecisionId::from("dec_bad"),
        task_id: TaskId::from("task_1"),
        run_id: RunId::from("run_1"),
        kind: DecisionKind::Accept,
        actor: "alice".to_string(),
        plan_version: 2,
        head_sha: "stale_head".to_string(),
        base_sha: "base_sha_1".to_string(),
        policy_digest: PolicyDigest::from("policy_v1"),
        evidence_digest: ev_digest.clone(),
        comment: None,
        expected_revision,
        created_at: Timestamp(1400),
    });
    assert!(matches!(
        BoardStore::apply_board_command(conn, &bad_dec, Timestamp(1400)),
        Err(StorageError::Transition(
            TransitionError::DecisionBindingMismatch { .. }
        ))
    ));

    let good_dec = BoardCommand::RecordDecision(RecordDecisionCommand {
        command_id: CommandId::from("cmd_dec_good"),
        decision_id: DecisionId::from("dec_good"),
        task_id: TaskId::from("task_1"),
        run_id: RunId::from("run_1"),
        kind: DecisionKind::Accept,
        actor: "alice".to_string(),
        plan_version: 2,
        head_sha: "head_sha_1".to_string(),
        base_sha: "base_sha_1".to_string(),
        policy_digest: PolicyDigest::from("policy_v1"),
        evidence_digest: ev_digest,
        comment: Some("LGTM".to_string()),
        expected_revision,
        created_at: Timestamp(1410),
    });
    BoardStore::apply_board_command(conn, &good_dec, Timestamp(1410)).expect("accept");
}

#[test]
fn test_board_commands_decisions_and_cost_ledger_roundtrip() {
    let mut conn = open_in_memory().expect("open memory db");
    initialize_schema(&conn).expect("init schema");
    seed_project_and_task(&mut conn);

    let proj = ProjectStore::get_project(&conn, &ProjectId::from("proj_1"))
        .unwrap()
        .expect("project exists");
    assert_eq!(proj.max_concurrent_runs, 2);

    for (idx, stage) in [
        TaskStage::Ready,
        TaskStage::InProgress,
        TaskStage::Acceptance,
    ]
    .into_iter()
    .enumerate()
    {
        let cmd = BoardCommand::MoveTaskStage(MoveTaskStageCommand {
            command_id: CommandId::from(format!("cmd_mv_{idx}")),
            task_id: TaskId::from("task_1"),
            target_stage: stage,
            actor: ActorRole::Human("alice".to_string()),
            expected_revision: Revision(u64::try_from(idx + 1).unwrap()),
            created_at: Timestamp(1100 + u64::try_from(idx).unwrap()),
        });
        BoardStore::apply_board_command(&mut conn, &cmd, Timestamp(1100)).expect("move stage");
    }

    let task_before_plan = TaskStore::get_task(&conn, &TaskId::from("task_1"))
        .unwrap()
        .unwrap();
    let plan_cmd = BoardCommand::UpdateTaskPlan(UpdateTaskPlanCommand {
        command_id: CommandId::from("cmd_plan_1"),
        task_id: TaskId::from("task_1"),
        plan_text: "1. Migrate schema\n2. Add CAS".to_string(),
        expected_revision: task_before_plan.revision,
        created_at: Timestamp(1200),
    });
    BoardStore::apply_board_command(&mut conn, &plan_cmd, Timestamp(1200)).expect("update plan");

    let task_now = TaskStore::get_task(&conn, &TaskId::from("task_1"))
        .unwrap()
        .unwrap();
    let agent_done = BoardCommand::MoveTaskStage(MoveTaskStageCommand {
        command_id: CommandId::from("cmd_agent_done"),
        task_id: TaskId::from("task_1"),
        target_stage: TaskStage::Done,
        actor: ActorRole::Agent("worker".to_string()),
        expected_revision: task_now.revision,
        created_at: Timestamp(1250),
    });
    assert!(matches!(
        BoardStore::apply_board_command(&mut conn, &agent_done, Timestamp(1250)),
        Err(StorageError::Transition(
            TransitionError::AgentCannotSelfAccept
        ))
    ));

    seed_session_and_attempts(&mut conn);
    let summary =
        AttemptStore::cost_summary_for_run(&conn, &RunId::from("run_1"), 0).expect("summary");
    assert_eq!(summary.known_micro_usd, 25_000);
    assert_eq!(summary.unknown_attempts, 1);

    assert_decision_binding_and_accept(&mut conn, task_now.revision);

    let final_task = TaskStore::get_task(&conn, &TaskId::from("task_1"))
        .unwrap()
        .unwrap();
    assert_eq!(final_task.stage, TaskStage::Done);
    assert!(DecisionStore::has_accept_for_task(&conn, &final_task.id).unwrap());
}

#[test]
fn test_cas_store_writes_reads_verifies_and_detects_corruption() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("runlark.db");
    let mut conn = open_sqlite(&db_path).expect("open db");
    initialize_schema(&conn).expect("init schema");
    seed_project_and_task(&mut conn);

    let session = Session {
        id: SessionId::from("sess_cas"),
        task_id: TaskId::from("task_1"),
        cwd: "/repo".to_string(),
        status: SessionStatus::Active,
        current_revision: Revision(1),
        active_run_id: Some(RunId::from("run_cas")),
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
    };
    SessionStore::create_session(&conn, &session).expect("create session");
    let run = Run {
        id: RunId::from("run_cas"),
        session_id: session.id,
        task_id: Some(TaskId::from("task_1")),
        project_id: Some(ProjectId::from("proj_1")),
        status: RunStatus::Running,
        worktree_path: "/repo".to_string(),
        ..Run::default()
    };
    SessionStore::save_run(&conn, &run).expect("save run");

    let cas = ArtifactCasStore::from_data_dir(dir.path());
    let payload = b"diff --git a/src/lib.rs b/src/lib.rs\n+pub fn hello() {}\n";
    let artifact = cas
        .put_bytes(
            &conn,
            ArtifactKind::Diff,
            "text/x-diff",
            payload,
            Timestamp(1500),
        )
        .expect("put bytes");

    assert_eq!(
        cas.blob_path(&artifact.digest_sha256),
        dir.path()
            .join("cas")
            .join("sha256")
            .join(artifact.digest_sha256.as_str())
    );
    assert_eq!(
        cas.get_bytes(&artifact.digest_sha256).expect("read back"),
        payload
    );

    let attach_cmd = BoardCommand::AttachArtifact(AttachArtifactCommand {
        command_id: CommandId::from("cmd_attach_1"),
        run_id: RunId::from("run_cas"),
        digest_sha256: artifact.digest_sha256.clone(),
        kind: ArtifactKind::Diff,
        media_type: "text/x-diff".to_string(),
        size_bytes: artifact.size_bytes,
        label: "git-diff".to_string(),
        created_at: Timestamp(1510),
    });
    BoardStore::apply_board_command(&mut conn, &attach_cmd, Timestamp(1510)).expect("attach");

    let board =
        BoardProjection::project_board(&conn, &ProjectId::from("proj_1")).expect("project board");
    assert_eq!(board.tasks.len(), 1);
    assert_eq!(board.executing_slots_used, 1);

    // Corrupting the blob on disk MUST trigger CorruptArtifact on read
    std::fs::write(cas.blob_path(&artifact.digest_sha256), b"tampered bytes")
        .expect("corrupt blob");
    assert!(matches!(
        cas.get_bytes(&artifact.digest_sha256),
        Err(StorageError::CorruptArtifact { .. })
    ));
}

#[test]
fn test_two_slot_scheduler_queries_exclude_waiting_and_preserve_fifo() {
    let conn = open_in_memory().expect("open db");
    initialize_schema(&conn).expect("init schema");

    let proj = runlark_domain::Project::new(
        ProjectId::from("proj_slots"),
        "Slots",
        "/repo",
        "main",
        BudgetPolicy::default(),
        PolicyDigest::from("pol_1"),
        Timestamp(1000),
    );
    ProjectStore::create_project(&conn, &proj).expect("create project");

    for (idx, status, ts) in [
        ("r_run", RunStatus::Running, 100),
        ("r_cancel", RunStatus::CancelRequested, 110),
        ("r_wait", RunStatus::WaitingForApproval, 120),
        ("r_q1", RunStatus::Queued, 130),
        ("r_q2", RunStatus::Queued, 140),
    ] {
        let sess = Session {
            id: SessionId::from(format!("s_{idx}")),
            task_id: TaskId::from(format!("t_{idx}")),
            cwd: "/repo".to_string(),
            status: SessionStatus::Active,
            current_revision: Revision(1),
            active_run_id: Some(RunId::from(idx)),
            created_at: Timestamp(ts),
            updated_at: Timestamp(ts),
        };
        SessionStore::create_session(&conn, &sess).expect("sess");
        SessionStore::save_run(
            &conn,
            &Run {
                id: RunId::from(idx),
                session_id: sess.id,
                project_id: Some(proj.id.clone()),
                task_id: Some(TaskId::from(format!("t_{idx}"))),
                status,
                worktree_path: format!("/wt/{idx}"),
                created_at: Timestamp(ts),
                updated_at: Timestamp(ts),
                ..Run::default()
            },
        )
        .expect("run");
    }

    // Running + CancelRequested = 2 slots; WaitingForApproval does NOT consume a slot!
    assert_eq!(BoardProjection::count_executing_runs(&conn).unwrap(), 2);
    assert_eq!(
        BoardProjection::count_executing_runs_for_project(&conn, &proj.id).unwrap(),
        2
    );

    let queued = BoardProjection::list_queued_runs_fifo(&conn).unwrap();
    let queued_ids: Vec<&str> = queued.iter().map(|r| r.id.as_ref()).collect();
    assert_eq!(queued_ids, vec!["r_q1", "r_q2"]);
}
