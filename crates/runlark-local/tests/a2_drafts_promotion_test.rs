use runlark_domain::{
    BoardCommand, BudgetPolicy, CommandId, CostSummary, CreateProjectCommand, CreateTaskCommand,
    CursorSeq, DraftId, DraftKind, DraftSyncState, EventSeq, LeaseEpoch, OfflineDraft,
    PolicyDigest, ProjectId, ProjectOwnershipMode, Revision, Run, RunId, RunStatus,
    RunnerEventInput, RunnerId, RunnerRunReport, ServerPromotionAck, Session, SessionId,
    SessionStatus, TaskId, TaskKind, Timestamp, TransitionError, UserId, WorkflowStage, Workspace,
    WorkspaceId, WorkspaceSnapshot,
};
use runlark_local::{
    BoardStore, DraftStore, RunnerBufferStore, SessionStore, StorageError, SyncOutboxEntry,
    SyncOutboxStatus, SyncOutboxStore, SyncStore, initialize_schema, open_in_memory,
};

fn setup_db() -> runlark_local::Connection {
    let conn = open_in_memory().expect("open in memory");
    initialize_schema(&conn).expect("schema");
    conn
}

fn seed_solo_project_with_run(conn: &mut runlark_local::Connection) -> Run {
    BoardStore::apply_board_command(
        conn,
        &BoardCommand::CreateProject(CreateProjectCommand {
            command_id: CommandId::from("cmd_proj_solo"),
            project_id: ProjectId::from("proj_solo"),
            name: "Solo".to_string(),
            repo_root: "/tmp/solo".to_string(),
            default_branch: "main".to_string(),
            budget_policy: BudgetPolicy::default(),
            policy_digest: PolicyDigest::from("pol_solo"),
            created_at: Timestamp(10),
        }),
        Timestamp(10),
    )
    .expect("project");
    BoardStore::apply_board_command(
        conn,
        &BoardCommand::CreateTask(CreateTaskCommand {
            command_id: CommandId::from("cmd_task_solo"),
            task_id: TaskId::from("task_solo"),
            project_id: ProjectId::from("proj_solo"),
            title: "Active".to_string(),
            description: String::new(),
            kind: TaskKind::QuickFix,
            base_ref: None,
            budget_override: None,
            created_at: Timestamp(11),
        }),
        Timestamp(11),
    )
    .expect("task");
    SessionStore::create_session(
        conn,
        &Session {
            id: SessionId::from("sess_solo"),
            task_id: TaskId::from("task_solo"),
            cwd: "/tmp/solo".to_string(),
            status: SessionStatus::Active,
            current_revision: Revision(1),
            active_run_id: Some(RunId::from("run_solo")),
            created_at: Timestamp(12),
            updated_at: Timestamp(12),
        },
    )
    .expect("session");
    let run = Run {
        id: RunId::from("run_solo"),
        session_id: SessionId::from("sess_solo"),
        status: RunStatus::WaitingForApproval,
        worktree_path: String::new(),
        revision: Revision(1),
        created_at: Timestamp(12),
        updated_at: Timestamp(12),
        task_id: Some(TaskId::from("task_solo")),
        project_id: Some(ProjectId::from("proj_solo")),
        workflow_stage: WorkflowStage::HumanAcceptance,
        task_kind: TaskKind::QuickFix,
        workflow_digest: None,
        base_sha: None,
        head_sha: None,
        plan_version: 1,
        policy_digest: Some(PolicyDigest::from("pol_solo")),
        budget_policy: BudgetPolicy::default(),
        cost_summary: CostSummary::default(),
        repair_attempts: 0,
        pr_url: None,
    };
    SessionStore::save_run(conn, &run).expect("run");
    run
}

#[test]
fn test_promote_project_to_team_atomic_blocks_active_runs() {
    let mut conn = setup_db();
    let mut run = seed_solo_project_with_run(&mut conn);
    let ack = ServerPromotionAck {
        workspace_id: WorkspaceId::from("ws_team"),
        project_id: ProjectId::from("proj_solo"),
        initial_cursor: CursorSeq(4),
        ownership_epoch: 1,
        acknowledged_at: Timestamp(30),
    };

    let err = SyncStore::promote_project_to_team_atomic(
        &mut conn,
        &ProjectId::from("proj_solo"),
        "http://127.0.0.1:8080",
        &ack,
        Timestamp(30),
    )
    .expect_err("active/waiting run must block promotion");
    assert!(matches!(
        err,
        StorageError::Transition(TransitionError::SoloToTeamBlockedByActiveRuns { active_runs: 1 })
    ));
    assert_eq!(
        SyncStore::get_ownership_mode(&conn, &ProjectId::from("proj_solo")).expect("mode"),
        ProjectOwnershipMode::SoloLocal
    );

    run.status = RunStatus::Completed;
    SessionStore::save_run(&conn, &run).expect("complete run");
    let binding = SyncStore::promote_project_to_team_atomic(
        &mut conn,
        &ProjectId::from("proj_solo"),
        "http://127.0.0.1:8080/",
        &ack,
        Timestamp(40),
    )
    .expect("promote");
    assert_eq!(binding.last_cursor_seq, CursorSeq(4));
    assert_eq!(
        SyncStore::get_ownership_mode(&conn, &ProjectId::from("proj_solo")).expect("mode"),
        ProjectOwnershipMode::TeamManaged {
            workspace_id: WorkspaceId::from("ws_team"),
            server_url: "http://127.0.0.1:8080".to_string(),
            ownership_epoch: 1,
        }
    );
}

#[test]
fn test_offline_drafts_and_sync_outbox_conflict_marking() {
    let mut conn = setup_db();
    BoardStore::apply_board_command(
        &mut conn,
        &BoardCommand::CreateProject(CreateProjectCommand {
            command_id: CommandId::from("cmd_pd"),
            project_id: ProjectId::from("proj_d"),
            name: "DraftProj".to_string(),
            repo_root: "/tmp/d".to_string(),
            default_branch: "main".to_string(),
            budget_policy: BudgetPolicy::default(),
            policy_digest: PolicyDigest::from("pol_d"),
            created_at: Timestamp(10),
        }),
        Timestamp(10),
    )
    .expect("proj");

    let draft = OfflineDraft {
        id: DraftId::from("draft_1"),
        project_id: ProjectId::from("proj_d"),
        task_id: None,
        kind: DraftKind::PlanEdit,
        payload_json: "{\"plan_text\":\"v2\"}".to_string(),
        base_revision: Some(Revision(1)),
        sync_state: DraftSyncState::PendingSync,
        created_at: Timestamp(20),
        updated_at: Timestamp(20),
    };
    DraftStore::save_draft(&conn, &draft).expect("save draft");

    let outbox = SyncOutboxEntry {
        command_id: CommandId::from("cmd_out_1"),
        workspace_id: WorkspaceId::from("ws_d"),
        project_id: Some(ProjectId::from("proj_d")),
        draft_id: Some(DraftId::from("draft_1")),
        command_type: "update_task_plan".to_string(),
        payload_json: "{}".to_string(),
        expected_revision: Some(Revision(1)),
        status: SyncOutboxStatus::Pending,
        last_error: None,
        conflict_server_revision: None,
        created_at: Timestamp(20),
        updated_at: Timestamp(20),
    };
    SyncOutboxStore::enqueue(&conn, &outbox).expect("enqueue");
    assert_eq!(
        SyncOutboxStore::list_pending_for_workspace(&conn, &WorkspaceId::from("ws_d"))
            .expect("pending")
            .len(),
        1
    );

    SyncOutboxStore::mark_conflict(
        &mut conn,
        &CommandId::from("cmd_out_1"),
        Some(Revision(4)),
        "expected revision 1, actual 4",
        Timestamp(30),
    )
    .expect("mark conflict");

    let updated_outbox = SyncOutboxStore::get(&conn, &CommandId::from("cmd_out_1"))
        .expect("get outbox")
        .expect("exists");
    assert_eq!(updated_outbox.status, SyncOutboxStatus::Conflict);
    assert_eq!(updated_outbox.conflict_server_revision, Some(Revision(4)));

    let updated_draft = DraftStore::get_draft(&conn, &DraftId::from("draft_1"))
        .expect("get draft")
        .expect("exists");
    assert_eq!(
        updated_draft.sync_state,
        DraftSyncState::Conflict {
            server_revision: Some(Revision(4)),
            reason: "expected revision 1, actual 4".to_string(),
        }
    );
}

#[test]
fn test_runner_buffer_store_preserves_order_and_marks_flushed() {
    let conn = setup_db();
    let report = RunnerRunReport {
        runner_id: RunnerId::from("runner_1"),
        lease_epoch: LeaseEpoch(2),
        status: Some(RunStatus::Running),
        workflow_stage: Some(WorkflowStage::Verification),
        head_sha: None,
        base_sha: None,
        events: vec![
            RunnerEventInput {
                seq: EventSeq(1),
                event_type: "step_start".to_string(),
                payload_json: "{}".to_string(),
                created_at: Timestamp(50),
            },
            RunnerEventInput {
                seq: EventSeq(2),
                event_type: "step_done".to_string(),
                payload_json: "{}".to_string(),
                created_at: Timestamp(51),
            },
        ],
    };
    RunnerBufferStore::buffer_report(
        &conn,
        &WorkspaceId::from("ws_d"),
        &RunId::from("run_buf"),
        &report,
    )
    .expect("buffer");
    let unflushed = RunnerBufferStore::list_unflushed(&conn, None).expect("unflushed");
    assert_eq!(unflushed.len(), 2);
    assert_eq!(unflushed[0].seq, EventSeq(1));
    assert_eq!(unflushed[1].seq, EventSeq(2));
    RunnerBufferStore::mark_flushed_up_to(&conn, &RunId::from("run_buf"), EventSeq(2))
        .expect("flush");
    assert!(
        RunnerBufferStore::list_unflushed(&conn, None)
            .expect("empty")
            .is_empty()
    );
}

#[test]
fn test_runner_buffer_allocate_in_tx_assigns_monotonic_seq_and_rejects_dup() {
    let mut mut_conn = setup_db();
    let ws = WorkspaceId::from("ws_tx");
    let rid = RunId::from("run_tx");
    let seq1 = runlark_local::with_immediate_tx(&mut mut_conn, |tx| {
        RunnerBufferStore::allocate_and_buffer_in_tx(
            tx,
            &ws,
            &rid,
            &RunnerRunReport {
                runner_id: RunnerId::from("runner_1"),
                lease_epoch: LeaseEpoch(1),
                status: None,
                workflow_stage: None,
                head_sha: None,
                base_sha: None,
                events: vec![RunnerEventInput {
                    seq: EventSeq(0),
                    event_type: "ev1".to_string(),
                    payload_json: "{}".to_string(),
                    created_at: Timestamp(10),
                }],
            },
        )
    })
    .expect("alloc 1");
    let seq2 = runlark_local::with_immediate_tx(&mut mut_conn, |tx| {
        RunnerBufferStore::allocate_and_buffer_in_tx(
            tx,
            &ws,
            &rid,
            &RunnerRunReport {
                runner_id: RunnerId::from("runner_1"),
                lease_epoch: LeaseEpoch(1),
                status: None,
                workflow_stage: None,
                head_sha: None,
                base_sha: None,
                events: vec![RunnerEventInput {
                    seq: EventSeq(0),
                    event_type: "ev2".to_string(),
                    payload_json: "{}".to_string(),
                    created_at: Timestamp(11),
                }],
            },
        )
    })
    .expect("alloc 2");
    assert_eq!((seq1, seq2), (EventSeq(1), EventSeq(2)));
    assert!(
        RunnerBufferStore::buffer_report(
            &mut_conn,
            &ws,
            &rid,
            &RunnerRunReport {
                runner_id: RunnerId::from("runner_1"),
                lease_epoch: LeaseEpoch(1),
                status: None,
                workflow_stage: None,
                head_sha: None,
                base_sha: None,
                events: vec![RunnerEventInput {
                    seq: EventSeq(1),
                    event_type: "dup".to_string(),
                    payload_json: "{}".to_string(),
                    created_at: Timestamp(12),
                }],
            }
        )
        .is_err()
    );
}

#[test]
fn test_promote_preserves_existing_workspace_cursor_and_initializes_new_at_zero() {
    let mut conn = setup_db();
    let ws = WorkspaceId::from("ws_existing");
    let snap = WorkspaceSnapshot {
        cursor_seq: CursorSeq(5),
        workspace: Workspace {
            id: ws.clone(),
            name: "Existing".to_string(),
            slug: "existing".to_string(),
            created_by: UserId::from("usr_owner"),
            revision: Revision(1),
            created_at: Timestamp(10),
            updated_at: Timestamp(10),
        },
        members: vec![],
        projects: vec![],
        tasks: vec![],
        runs: vec![],
        attempts: vec![],
        decisions: vec![],
        comments: vec![],
        artifacts: vec![],
        runners: vec![],
    };
    SyncStore::apply_snapshot_atomic(&mut conn, "http://127.0.0.1:8080", &snap, Timestamp(20))
        .expect("apply snapshot");

    BoardStore::apply_board_command(
        &mut conn,
        &BoardCommand::CreateProject(CreateProjectCommand {
            command_id: CommandId::from("cmd_proj_second"),
            project_id: ProjectId::from("proj_second"),
            name: "Second".to_string(),
            repo_root: "/tmp/second".to_string(),
            default_branch: "main".to_string(),
            budget_policy: BudgetPolicy::default(),
            policy_digest: PolicyDigest::from("pol_second"),
            created_at: Timestamp(30),
        }),
        Timestamp(30),
    )
    .expect("create second");

    let ack = ServerPromotionAck {
        workspace_id: ws.clone(),
        project_id: ProjectId::from("proj_second"),
        initial_cursor: CursorSeq(12),
        ownership_epoch: 1,
        acknowledged_at: Timestamp(40),
    };
    let binding = SyncStore::promote_project_to_team_atomic(
        &mut conn,
        &ProjectId::from("proj_second"),
        "http://127.0.0.1:8080",
        &ack,
        Timestamp(40),
    )
    .expect("promote second");
    assert_eq!(binding.last_cursor_seq, CursorSeq(12));
    assert_eq!(
        SyncStore::get_workspace_cursor(&conn, &ws).expect("ws cursor preserved"),
        CursorSeq(5)
    );

    BoardStore::apply_board_command(
        &mut conn,
        &BoardCommand::CreateProject(CreateProjectCommand {
            command_id: CommandId::from("cmd_proj_fresh"),
            project_id: ProjectId::from("proj_fresh"),
            name: "Fresh".to_string(),
            repo_root: "/tmp/fresh".to_string(),
            default_branch: "main".to_string(),
            budget_policy: BudgetPolicy::default(),
            policy_digest: PolicyDigest::from("pol_fresh"),
            created_at: Timestamp(50),
        }),
        Timestamp(50),
    )
    .expect("create fresh");
    let ws_new = WorkspaceId::from("ws_unseen");
    SyncStore::promote_project_to_team_atomic(
        &mut conn,
        &ProjectId::from("proj_fresh"),
        "http://127.0.0.1:8080",
        &ServerPromotionAck {
            workspace_id: ws_new.clone(),
            project_id: ProjectId::from("proj_fresh"),
            initial_cursor: CursorSeq(9),
            ownership_epoch: 1,
            acknowledged_at: Timestamp(60),
        },
        Timestamp(60),
    )
    .expect("promote fresh");
    assert_eq!(
        SyncStore::get_workspace_cursor(&conn, &ws_new).expect("new ws starts at 0 for snapshot"),
        CursorSeq(0)
    );
}

#[test]
fn test_outbox_status_transitions_roll_back_on_draft_failure() {
    let mut conn = setup_db();
    BoardStore::apply_board_command(
        &mut conn,
        &BoardCommand::CreateProject(CreateProjectCommand {
            command_id: CommandId::from("cmd_atomic"),
            project_id: ProjectId::from("proj_atomic"),
            name: "Atomic".to_string(),
            repo_root: "/tmp/atomic".to_string(),
            default_branch: "main".to_string(),
            budget_policy: BudgetPolicy::default(),
            policy_digest: PolicyDigest::from("pol_atomic"),
            created_at: Timestamp(10),
        }),
        Timestamp(10),
    )
    .expect("proj");

    DraftStore::save_draft(
        &conn,
        &OfflineDraft {
            id: DraftId::from("draft_atomic"),
            project_id: ProjectId::from("proj_atomic"),
            task_id: None,
            kind: DraftKind::PlanEdit,
            payload_json: "{\"plan_text\":\"v1\"}".to_string(),
            base_revision: Some(Revision(1)),
            sync_state: DraftSyncState::PendingSync,
            created_at: Timestamp(20),
            updated_at: Timestamp(20),
        },
    )
    .expect("draft");
    SyncOutboxStore::enqueue(
        &conn,
        &SyncOutboxEntry {
            command_id: CommandId::from("cmd_atomic_out"),
            workspace_id: WorkspaceId::from("ws_atomic"),
            project_id: Some(ProjectId::from("proj_atomic")),
            draft_id: Some(DraftId::from("draft_atomic")),
            command_type: "update_task_plan".to_string(),
            payload_json: "{}".to_string(),
            expected_revision: Some(Revision(1)),
            status: SyncOutboxStatus::Pending,
            last_error: None,
            conflict_server_revision: None,
            created_at: Timestamp(20),
            updated_at: Timestamp(20),
        },
    )
    .expect("enqueue");

    conn.execute_batch(concat!(
        "CREATE TEMP TRIGGER block_draft_update BEFORE UPDATE ON offline_drafts ",
        "BEGIN SELECT RAISE(ABORT, 'simulated draft failure'); END;",
    ))
    .expect("trigger");

    assert!(
        SyncOutboxStore::mark_acked(&mut conn, &CommandId::from("cmd_atomic_out"), Timestamp(30))
            .is_err()
    );
    assert_eq!(
        SyncOutboxStore::get(&conn, &CommandId::from("cmd_atomic_out"))
            .unwrap()
            .unwrap()
            .status,
        SyncOutboxStatus::Pending
    );
    assert!(
        SyncOutboxStore::mark_conflict(
            &mut conn,
            &CommandId::from("cmd_atomic_out"),
            Some(Revision(2)),
            "conflict",
            Timestamp(31),
        )
        .is_err()
    );
    assert_eq!(
        SyncOutboxStore::get(&conn, &CommandId::from("cmd_atomic_out"))
            .unwrap()
            .unwrap()
            .status,
        SyncOutboxStatus::Pending
    );
    assert!(
        SyncOutboxStore::mark_failed(
            &mut conn,
            &CommandId::from("cmd_atomic_out"),
            "failed",
            Timestamp(32),
        )
        .is_err()
    );
    assert_eq!(
        SyncOutboxStore::get(&conn, &CommandId::from("cmd_atomic_out"))
            .unwrap()
            .unwrap()
            .status,
        SyncOutboxStatus::Pending
    );
}
