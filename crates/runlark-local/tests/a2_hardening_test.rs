use runlark_domain::{
    BudgetPolicy, CommandId, CommentId, CostSummary, CursorSeq, DraftId, DraftKind, DraftSyncState,
    LeaseEpoch, OfflineDraft, PolicyDigest, Project, ProjectId, Revision, Run, RunId, RunOwnership,
    RunStatus, RunnerId, ServerPromotionAck, Session, SessionId, SessionStatus, Task, TaskComment,
    TaskId, TaskKind, TaskStage, TeamProjectSpec, TeamRunEntry, Timestamp, UserId, WorkflowStage,
    Workspace, WorkspaceId, WorkspaceSnapshot,
};
use runlark_local::{
    BoardProjection, CommentStore, DraftStore, ProjectStore, ReconciliationEngine, RunQueries,
    SessionStore, StorageError, SyncOutboxEntry, SyncOutboxStatus, SyncOutboxStore, SyncStore,
    TeamAuthSession, TeamAuthStore, initialize_schema, open_in_memory,
};

fn setup_db() -> runlark_local::Connection {
    let conn = open_in_memory().expect("open in-memory db");
    initialize_schema(&conn).expect("init schema");
    conn
}

fn seed_project_and_task(conn: &runlark_local::Connection, pid: &str, tid: &str) {
    let proj = Project::new(
        ProjectId::from(pid),
        "Hardening",
        "/tmp/repo",
        "main",
        BudgetPolicy::default(),
        PolicyDigest::from("pol_1"),
        Timestamp(100),
    );
    ProjectStore::create_project(conn, &proj).expect("create project");
    let task = Task {
        id: TaskId::from(tid),
        project_id: proj.id,
        title: "Task".to_string(),
        description: "Desc".to_string(),
        kind: TaskKind::QuickFix,
        stage: TaskStage::Inbox,
        workflow_stage: WorkflowStage::Implementation,
        plan_text: None,
        plan_version: 0,
        base_ref: None,
        head_ref: None,
        pr_url: None,
        active_session_id: None,
        active_run_id: None,
        latest_run_id: None,
        budget_override: None,
        revision: Revision(1),
        created_at: Timestamp(101),
        updated_at: Timestamp(101),
    };
    runlark_local::TaskStore::create_task(conn, &task).expect("create task");
}

#[test]
fn sync_outbox_rejects_payload_conflict_and_preserves_acked_and_synced_states() {
    let mut conn = setup_db();
    seed_project_and_task(&conn, "prj_1", "tsk_1");

    let draft = OfflineDraft {
        id: DraftId::from("drf_1"),
        project_id: ProjectId::from("prj_1"),
        task_id: Some(TaskId::from("tsk_1")),
        kind: DraftKind::TaskComment,
        payload_json: r#"{"body":"hello"}"#.to_string(),
        base_revision: None,
        sync_state: DraftSyncState::PendingSync,
        created_at: Timestamp(10),
        updated_at: Timestamp(10),
    };
    DraftStore::save_draft(&conn, &draft).expect("save draft");

    // Reusing draft_id with a different kind or task_id must fail cleanly.
    let bad_reuse = OfflineDraft {
        kind: DraftKind::PlanEdit,
        ..draft.clone()
    };
    assert!(matches!(
        DraftStore::save_draft(&conn, &bad_reuse),
        Err(StorageError::InvalidState(_))
    ));

    let entry = SyncOutboxEntry {
        command_id: CommandId::from("cmd_out_1"),
        workspace_id: WorkspaceId::from("ws_1"),
        project_id: Some(ProjectId::from("prj_1")),
        draft_id: Some(draft.id.clone()),
        command_type: "add_comment".to_string(),
        payload_json: r#"{"body":"hello"}"#.to_string(),
        expected_revision: None,
        status: SyncOutboxStatus::Pending,
        last_error: None,
        conflict_server_revision: None,
        created_at: Timestamp(10),
        updated_at: Timestamp(10),
    };
    SyncOutboxStore::enqueue(&conn, &entry).expect("enqueue first");
    SyncOutboxStore::enqueue(&conn, &entry).expect("identical replay is idempotent");

    let mutated = SyncOutboxEntry {
        payload_json: r#"{"body":"changed"}"#.to_string(),
        ..entry.clone()
    };
    assert!(matches!(
        SyncOutboxStore::enqueue(&conn, &mutated),
        Err(StorageError::CommandPayloadConflict(_))
    ));

    SyncOutboxStore::mark_acked(&mut conn, &entry.command_id, Timestamp(20)).expect("ack");
    SyncOutboxStore::mark_failed(&mut conn, &entry.command_id, "late 403", Timestamp(30))
        .expect("late mark_failed must not regress acked");
    SyncOutboxStore::mark_conflict(
        &mut conn,
        &entry.command_id,
        Some(Revision(5)),
        "late 409",
        Timestamp(31),
    )
    .expect("late mark_conflict must not regress acked");

    let after = SyncOutboxStore::get(&conn, &entry.command_id)
        .expect("get outbox")
        .expect("exists");
    assert_eq!(after.status, SyncOutboxStatus::Acked);
    let after_draft = DraftStore::get_draft(&conn, &draft.id)
        .expect("get draft")
        .expect("exists");
    assert_eq!(after_draft.sync_state, DraftSyncState::Synced);
    assert!(matches!(
        DraftStore::mark_pending_sync(&conn, &draft.id, None, None, Timestamp(40)),
        Err(StorageError::InvalidState(_))
    ));

    // CommentStore rejects conflicting content for a duplicate comment_id.
    let comment = TaskComment {
        id: CommentId::from("cmt_1"),
        task_id: TaskId::from("tsk_1"),
        run_id: None,
        author_user_id: UserId::from("usr_1"),
        author_display_name: "Alice".to_string(),
        body: "original".to_string(),
        created_at: Timestamp(50),
    };
    CommentStore::upsert_comment(&conn, &comment).expect("insert comment");
    CommentStore::upsert_comment(&conn, &comment).expect("identical replay ok");
    let modified = TaskComment {
        body: "tampered".to_string(),
        ..comment
    };
    assert!(matches!(
        CommentStore::upsert_comment(&conn, &modified),
        Err(StorageError::InvalidState(_))
    ));
}

#[test]
fn resolve_for_workspace_isolates_servers_and_redacts_tokens() {
    let conn = setup_db();
    let sess_a = TeamAuthSession {
        server_url: "http://server-a.local".to_string(),
        user_id: UserId::from("usr_a"),
        display_name: "Alice".to_string(),
        bearer_token: "super_secret_token_alpha".to_string(),
        default_workspace_id: Some(WorkspaceId::from("ws_a")),
        runner_id: Some(RunnerId::from("rnr_a")),
        runner_name: Some("Alpha".to_string()),
        updated_at: Timestamp(100),
    };
    assert!(!format!("{sess_a:?}").contains("super_secret_token_alpha"));
    assert!(format!("{sess_a:?}").contains("[REDACTED]"));

    TeamAuthStore::save_session(&conn, &sess_a).expect("save server A session");
    SyncStore::ensure_workspace_placeholder(
        &conn,
        "http://server-b.local",
        &WorkspaceId::from("ws_b"),
        Timestamp(110),
    )
    .expect("seed ws_b on logged-out server B");

    // ws_b belongs to server-b.local which has no saved session: must NEVER leak server-a's session.
    let resolved_b = TeamAuthStore::resolve_for_workspace(&conn, Some(&WorkspaceId::from("ws_b")))
        .expect("resolve ws_b");
    assert_eq!(resolved_b, None);

    // replace_session clears NULL fields accurately on rollback.
    let cleared = TeamAuthSession {
        default_workspace_id: None,
        runner_id: None,
        runner_name: None,
        ..sess_a.clone()
    };
    TeamAuthStore::replace_session(&conn, &cleared).expect("replace session");
    let reloaded = TeamAuthStore::get_by_server(&conn, "http://server-a.local")
        .expect("get")
        .expect("exists");
    assert_eq!(reloaded.default_workspace_id, None);
    assert_eq!(reloaded.runner_id, None);

    // set_runner_identity ignores wrong user_id and populates default_workspace_id for matching user.
    TeamAuthStore::set_runner_identity(
        &conn,
        "http://server-a.local",
        &UserId::from("usr_wrong"),
        &WorkspaceId::from("ws_a"),
        &RunnerId::from("rnr_wrong"),
        "Wrong",
        Timestamp(120),
    )
    .expect("wrong user ignored");
    assert_eq!(
        TeamAuthStore::get_by_server(&conn, "http://server-a.local")
            .unwrap()
            .unwrap()
            .runner_id,
        None
    );

    TeamAuthStore::set_runner_identity(
        &conn,
        "http://server-a.local",
        &UserId::from("usr_a"),
        &WorkspaceId::from("ws_a"),
        &RunnerId::from("rnr_a2"),
        "Alice Mac",
        Timestamp(130),
    )
    .expect("matching user updated");
    let updated = TeamAuthStore::get_by_server(&conn, "http://server-a.local")
        .unwrap()
        .unwrap();
    assert_eq!(updated.runner_id, Some(RunnerId::from("rnr_a2")));
    assert_eq!(
        updated.default_workspace_id,
        Some(WorkspaceId::from("ws_a"))
    );
}

fn make_team_run_entry(id: &str, sid: &str, pid: &ProjectId, runner: &str) -> TeamRunEntry {
    TeamRunEntry {
        run: Run {
            id: RunId::from(id),
            session_id: SessionId::from(sid),
            status: RunStatus::Running,
            worktree_path: String::new(),
            revision: Revision(1),
            created_at: Timestamp(100),
            updated_at: Timestamp(100),
            task_id: None,
            project_id: Some(pid.clone()),
            workflow_stage: WorkflowStage::Implementation,
            task_kind: TaskKind::QuickFix,
            workflow_digest: None,
            base_sha: None,
            head_sha: None,
            plan_version: 0,
            policy_digest: None,
            budget_policy: BudgetPolicy::default(),
            cost_summary: CostSummary::default(),
            repair_attempts: 0,
            pr_url: None,
        },
        ownership: RunOwnership {
            assigned_runner_id: Some(RunnerId::from(runner)),
            lease_epoch: LeaseEpoch(1),
            runner_unreachable: false,
        },
    }
}

#[test]
fn remote_teammate_runs_do_not_consume_local_slots_or_interrupt_on_restart() {
    let mut conn = setup_db();
    let ws = Workspace {
        id: WorkspaceId::from("ws_team"),
        name: "Team".to_string(),
        slug: "team".to_string(),
        created_by: UserId::from("usr_me"),
        revision: Revision(1),
        created_at: Timestamp(100),
        updated_at: Timestamp(100),
    };
    let proj = TeamProjectSpec {
        project_id: ProjectId::from("prj_team"),
        workspace_id: ws.id.clone(),
        name: "Team Repo".to_string(),
        default_branch: "main".to_string(),
        max_concurrent_runs: 2,
        budget_policy: BudgetPolicy::default(),
        policy_digest: PolicyDigest::from("pol_team"),
        ownership_epoch: 1,
        revision: Revision(1),
        created_at: Timestamp(100),
        updated_at: Timestamp(100),
    };
    let proj_id = proj.project_id.clone();
    let snap = WorkspaceSnapshot {
        workspace: ws,
        cursor_seq: CursorSeq(5),
        members: vec![],
        runners: vec![],
        projects: vec![proj],
        tasks: vec![],
        runs: vec![
            make_team_run_entry("run_mate_1", "sess_m1", &proj_id, "rnr_teammate"),
            make_team_run_entry("run_mate_2", "sess_m2", &proj_id, "rnr_teammate"),
            make_team_run_entry("run_mine_1", "sess_me1", &proj_id, "rnr_mine"),
        ],
        attempts: vec![],
        decisions: vec![],
        comments: vec![],
        artifacts: vec![],
    };
    TeamAuthStore::save_session(
        &conn,
        &TeamAuthSession {
            server_url: "http://team.local".to_string(),
            user_id: UserId::from("usr_me"),
            display_name: "Me".to_string(),
            bearer_token: "tok".to_string(),
            default_workspace_id: Some(WorkspaceId::from("ws_team")),
            runner_id: Some(RunnerId::from("rnr_mine")),
            runner_name: Some("Mine".to_string()),
            updated_at: Timestamp(100),
        },
    )
    .expect("save auth");
    SyncStore::apply_snapshot_atomic(&mut conn, "http://team.local", &snap, Timestamp(100))
        .expect("apply snapshot");

    // Only the 1 locally leased run consumes a local slot; 2 remote teammates' runs do not.
    let executing = RunQueries::list_executing_runs(&conn).expect("list executing");
    assert_eq!(executing.len(), 1);
    assert_eq!(executing[0].id, RunId::from("run_mine_1"));
    assert_eq!(BoardProjection::count_executing_runs(&conn).unwrap(), 1);

    // Startup reconciliation interrupts only run_mine_1, leaving run_mate_1 & run_mate_2 Running.
    let inflight =
        ReconciliationEngine::reconcile_on_startup(&mut conn, Timestamp(200)).expect("reconcile");
    assert_eq!(inflight.active_runs.len(), 1);
    assert_eq!(inflight.active_runs[0].0, RunId::from("run_mine_1"));
    assert_eq!(
        SessionStore::get_run(&conn, &RunId::from("run_mate_1"))
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Running
    );
    assert_eq!(
        SessionStore::get_run(&conn, &RunId::from("run_mine_1"))
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Interrupted
    );

    // Stale snapshot with lower cursor_seq must NOT regress cursor_seq or overwrite state.
    let mut stale = snap;
    stale.cursor_seq = CursorSeq(2);
    SyncStore::apply_snapshot_atomic(&mut conn, "http://team.local", &stale, Timestamp(300))
        .expect("stale snapshot ignored");
    assert_eq!(
        SyncStore::get_workspace_cursor(&conn, &WorkspaceId::from("ws_team")).unwrap(),
        CursorSeq(5)
    );
}

#[test]
fn session_linked_a0_active_run_blocks_solo_to_team_promotion() {
    let mut conn = setup_db();
    seed_project_and_task(&conn, "prj_a0", "tsk_a0");

    let session = Session {
        id: SessionId::from("sess_a0"),
        task_id: TaskId::from("tsk_a0"),
        cwd: "/tmp/repo".to_string(),
        status: SessionStatus::Active,
        current_revision: Revision(1),
        active_run_id: Some(RunId::from("run_a0_only")),
        created_at: Timestamp(100),
        updated_at: Timestamp(100),
    };
    SessionStore::create_session(&conn, &session).expect("create session");
    let a0_run = Run {
        id: RunId::from("run_a0_only"),
        session_id: session.id,
        status: RunStatus::Running,
        worktree_path: "/tmp/repo".to_string(),
        revision: Revision(1),
        created_at: Timestamp(105),
        updated_at: Timestamp(105),
        task_id: None,
        project_id: None,
        workflow_stage: WorkflowStage::Implementation,
        task_kind: TaskKind::QuickFix,
        workflow_digest: None,
        base_sha: None,
        head_sha: None,
        plan_version: 0,
        policy_digest: None,
        budget_policy: BudgetPolicy::default(),
        cost_summary: CostSummary::default(),
        repair_attempts: 0,
        pr_url: None,
    };
    SessionStore::save_run(&conn, &a0_run).expect("save A0 run without project_id/task_id");

    assert!(
        SyncStore::validate_project_promotion_eligible(&conn, &ProjectId::from("prj_a0")).is_err()
    );
    let ack = ServerPromotionAck {
        workspace_id: WorkspaceId::from("ws_a0"),
        project_id: ProjectId::from("prj_a0"),
        ownership_epoch: 1,
        initial_cursor: CursorSeq(0),
        acknowledged_at: Timestamp(200),
    };
    assert!(
        SyncStore::promote_project_to_team_atomic(
            &mut conn,
            &ProjectId::from("prj_a0"),
            "http://team.local",
            &ack,
            Timestamp(200)
        )
        .is_err()
    );
}
