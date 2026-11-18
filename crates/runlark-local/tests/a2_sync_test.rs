use runlark_domain::{
    ArtifactDigest, ArtifactKind, ArtifactRef, Attempt, AttemptId, AttemptOutcome, BoardCommand,
    BudgetPolicy, CommandId, CommentId, CostSummary, CreateProjectCommand, CursorSeq, Decision,
    DecisionId, DecisionKind, LeaseEpoch, PolicyDigest, ProjectId, ProjectOwnershipMode, Revision,
    Run, RunId, RunOwnership, RunStatus, RunnerId, RunnerRegistration, RunnerStatus, Task,
    TaskComment, TaskId, TaskKind, TaskStage, TeamProjectSpec, TeamRunArtifactEntry, TeamRunEntry,
    Timestamp, UserId, WorkflowStage, Workspace, WorkspaceEventEnvelope, WorkspaceId,
    WorkspaceMember, WorkspaceRole, WorkspaceSnapshot, WorkspaceSyncEvent,
};
use runlark_local::{
    BoardProjection, BoardStore, CommentStore, ProjectStore, SessionStore, StorageError, SyncStore,
    TaskStore, TeamAuthSession, TeamAuthStore, initialize_schema, open_in_memory,
};

fn setup_db() -> runlark_local::Connection {
    let conn = open_in_memory().expect("open in memory");
    initialize_schema(&conn).expect("schema");
    conn
}

fn sample_workspace(wid: &str) -> Workspace {
    Workspace {
        id: WorkspaceId::from(wid),
        name: "Core Team".to_string(),
        slug: "core-team".to_string(),
        created_by: UserId::from("usr_alice"),
        revision: Revision(1),
        created_at: Timestamp(100),
        updated_at: Timestamp(100),
    }
}

fn sample_project_spec(pid: &str, wid: &str) -> TeamProjectSpec {
    TeamProjectSpec {
        project_id: ProjectId::from(pid),
        workspace_id: WorkspaceId::from(wid),
        name: "Runlark".to_string(),
        default_branch: "main".to_string(),
        max_concurrent_runs: 2,
        budget_policy: BudgetPolicy::default(),
        policy_digest: PolicyDigest::from("pol_1"),
        ownership_epoch: 1,
        revision: Revision(1),
        created_at: Timestamp(100),
        updated_at: Timestamp(100),
    }
}

fn sample_task(tid: &str, pid: &str, rev: u64) -> Task {
    Task {
        id: TaskId::from(tid),
        project_id: ProjectId::from(pid),
        title: "Synced task".to_string(),
        description: "Desc".to_string(),
        kind: TaskKind::Standard,
        stage: TaskStage::InProgress,
        workflow_stage: WorkflowStage::Implementation,
        plan_text: Some("Plan v1".to_string()),
        plan_version: 1,
        base_ref: Some("main".to_string()),
        head_ref: None,
        pr_url: None,
        active_session_id: Some(runlark_domain::SessionId::from("sess_1")),
        active_run_id: Some(RunId::from("run_1")),
        latest_run_id: Some(RunId::from("run_1")),
        budget_override: None,
        revision: Revision(rev),
        created_at: Timestamp(110),
        updated_at: Timestamp(110),
    }
}

fn sample_run(rid: &str, pid: &str, tid: &str, status: RunStatus) -> Run {
    Run {
        id: RunId::from(rid),
        session_id: runlark_domain::SessionId::from("sess_1"),
        status,
        worktree_path: String::new(),
        revision: Revision(1),
        created_at: Timestamp(120),
        updated_at: Timestamp(120),
        task_id: Some(TaskId::from(tid)),
        project_id: Some(ProjectId::from(pid)),
        workflow_stage: WorkflowStage::Implementation,
        task_kind: TaskKind::Standard,
        workflow_digest: None,
        base_sha: Some("1111111111111111111111111111111111111111".to_string()),
        head_sha: Some("2222222222222222222222222222222222222222".to_string()),
        plan_version: 1,
        policy_digest: Some(PolicyDigest::from("pol_1")),
        budget_policy: BudgetPolicy::default(),
        cost_summary: CostSummary::default(),
        repair_attempts: 0,
        pr_url: None,
    }
}

fn build_full_snapshot() -> WorkspaceSnapshot {
    let digest = ArtifactDigest::parse("a".repeat(64)).expect("digest");
    WorkspaceSnapshot {
        cursor_seq: CursorSeq(7),
        workspace: sample_workspace("ws_1"),
        members: vec![WorkspaceMember {
            workspace_id: WorkspaceId::from("ws_1"),
            user_id: UserId::from("usr_alice"),
            display_name: "Alice".to_string(),
            role: WorkspaceRole::Owner,
            joined_at: Timestamp(100),
        }],
        projects: vec![sample_project_spec("proj_1", "ws_1")],
        tasks: vec![sample_task("task_1", "proj_1", 2)],
        runs: vec![TeamRunEntry {
            run: sample_run("run_1", "proj_1", "task_1", RunStatus::Running),
            ownership: RunOwnership {
                assigned_runner_id: Some(RunnerId::from("runner_a")),
                lease_epoch: LeaseEpoch(3),
                runner_unreachable: false,
            },
        }],
        attempts: vec![Attempt {
            id: AttemptId::from("att_1"),
            run_id: RunId::from("run_1"),
            attempt_no: 1,
            workflow_stage: WorkflowStage::Implementation,
            started_at: Timestamp(125),
            ended_at: Some(Timestamp(126)),
            outcome: AttemptOutcome::Succeeded,
            cost_micro_usd: Some(500),
            unknown_cost_reason: None,
            repair_reason: None,
        }],
        decisions: vec![Decision {
            id: DecisionId::from("dec_1"),
            task_id: TaskId::from("task_1"),
            run_id: RunId::from("run_1"),
            kind: DecisionKind::ApprovePlan,
            actor: "Alice".to_string(),
            plan_version: 1,
            head_sha: "2222".to_string(),
            base_sha: "1111".to_string(),
            policy_digest: PolicyDigest::from("pol_1"),
            evidence_digest: digest.clone(),
            comment: Some("ok".to_string()),
            created_at: Timestamp(130),
        }],
        comments: vec![TaskComment {
            id: CommentId::from("cmt_1"),
            task_id: TaskId::from("task_1"),
            run_id: Some(RunId::from("run_1")),
            author_user_id: UserId::from("usr_alice"),
            author_display_name: "Alice".to_string(),
            body: "Initial team comment".to_string(),
            created_at: Timestamp(135),
        }],
        artifacts: vec![TeamRunArtifactEntry {
            run_id: RunId::from("run_1"),
            artifact: ArtifactRef {
                digest_sha256: digest,
                kind: ArtifactKind::EvidenceBundle,
                media_type: "application/json".to_string(),
                size_bytes: 128,
                created_at: Timestamp(132),
            },
            label: "evidence_bundle".to_string(),
        }],
        runners: vec![RunnerRegistration {
            id: RunnerId::from("runner_a"),
            workspace_id: WorkspaceId::from("ws_1"),
            owner_user_id: UserId::from("usr_alice"),
            name: "Alice Laptop".to_string(),
            status: RunnerStatus::Online,
            last_heartbeat_at: Timestamp(140),
            created_at: Timestamp(100),
        }],
    }
}

#[test]
fn test_apply_snapshot_atomic_preserves_local_paths_and_projects_board() {
    let mut conn = setup_db();
    BoardStore::apply_board_command(
        &mut conn,
        &BoardCommand::CreateProject(CreateProjectCommand {
            command_id: CommandId::from("cmd_p1"),
            project_id: ProjectId::from("proj_1"),
            name: "Local Name".to_string(),
            repo_root: "/local/dev/repo".to_string(),
            default_branch: "main".to_string(),
            budget_policy: BudgetPolicy::default(),
            policy_digest: PolicyDigest::from("pol_1"),
            created_at: Timestamp(50),
        }),
        Timestamp(50),
    )
    .expect("create local project");

    let snap = build_full_snapshot();
    SyncStore::apply_snapshot_atomic(&mut conn, "http://127.0.0.1:8080/", &snap, Timestamp(150))
        .expect("apply snapshot");

    let proj = ProjectStore::get_project(&conn, &ProjectId::from("proj_1"))
        .expect("get proj")
        .expect("proj exists");
    assert_eq!(proj.repo_root, "/local/dev/repo");
    assert_eq!(proj.name, "Runlark");

    SessionStore::update_run_worktree(
        &conn,
        &RunId::from("run_1"),
        "/local/wt/run_1",
        Timestamp(155),
    )
    .expect("set local wt");
    SyncStore::apply_snapshot_atomic(&mut conn, "http://127.0.0.1:8080", &snap, Timestamp(160))
        .expect("re-apply snapshot");
    let run_after = SessionStore::get_run(&conn, &RunId::from("run_1"))
        .expect("get run")
        .expect("run exists");
    assert_eq!(run_after.worktree_path, "/local/wt/run_1");

    let board = BoardProjection::project_board(&conn, &ProjectId::from("proj_1")).expect("board");
    assert_eq!(board.last_cursor_seq, CursorSeq(7));
    assert_eq!(
        board.ownership_mode,
        ProjectOwnershipMode::TeamManaged {
            workspace_id: WorkspaceId::from("ws_1"),
            server_url: "http://127.0.0.1:8080".to_string(),
            ownership_epoch: 1,
        }
    );
    assert_eq!(board.members.len(), 1);
    assert_eq!(board.runners.len(), 1);
    assert_eq!(board.tasks[0].comments.len(), 1);
}

fn task_upsert_envelope(seq: u64, title: &str, ts: u64) -> WorkspaceEventEnvelope {
    let mut updated_task = sample_task("task_2", "proj_2", 2);
    updated_task.title = title.to_string();
    WorkspaceEventEnvelope {
        cursor_seq: CursorSeq(seq),
        workspace_id: WorkspaceId::from("ws_2"),
        project_id: Some(ProjectId::from("proj_2")),
        aggregate_type: "task".to_string(),
        aggregate_id: "task_2".to_string(),
        event: WorkspaceSyncEvent::TaskUpserted { task: updated_task },
        actor_user_id: UserId::from("usr_alice"),
        created_at: Timestamp(ts),
    }
}

fn comment_envelope(seq: u64, cid: &str, body: &str, ts: u64) -> WorkspaceEventEnvelope {
    WorkspaceEventEnvelope {
        cursor_seq: CursorSeq(seq),
        workspace_id: WorkspaceId::from("ws_2"),
        project_id: Some(ProjectId::from("proj_2")),
        aggregate_type: "comment".to_string(),
        aggregate_id: cid.to_string(),
        event: WorkspaceSyncEvent::CommentAppended {
            comment: TaskComment {
                id: CommentId::from(cid),
                task_id: TaskId::from("task_2"),
                run_id: None,
                author_user_id: UserId::from("usr_alice"),
                author_display_name: "Alice".to_string(),
                body: body.to_string(),
                created_at: Timestamp(ts),
            },
        },
        actor_user_id: UserId::from("usr_alice"),
        created_at: Timestamp(ts),
    }
}

#[test]
fn test_apply_events_batch_atomic_contiguous_and_gap_rollback() {
    let mut conn = setup_db();
    let snap = WorkspaceSnapshot {
        cursor_seq: CursorSeq(2),
        workspace: sample_workspace("ws_2"),
        members: vec![],
        projects: vec![sample_project_spec("proj_2", "ws_2")],
        tasks: vec![sample_task("task_2", "proj_2", 1)],
        runs: vec![],
        attempts: vec![],
        decisions: vec![],
        comments: vec![],
        artifacts: vec![],
        runners: vec![],
    };
    SyncStore::apply_snapshot_atomic(&mut conn, "http://127.0.0.1:8080", &snap, Timestamp(200))
        .expect("snapshot");

    let gap_batch = vec![
        task_upsert_envelope(3, "Should roll back", 210),
        comment_envelope(5, "cmt_gap", "Gap comment", 220),
    ];
    let err = SyncStore::apply_events_batch_atomic(
        &mut conn,
        &WorkspaceId::from("ws_2"),
        "http://127.0.0.1:8080",
        &gap_batch,
        Timestamp(225),
    )
    .expect_err("gap must fail");
    assert!(matches!(
        err,
        StorageError::CursorGap {
            expected: CursorSeq(4),
            actual: CursorSeq(5),
        }
    ));
    assert_eq!(
        SyncStore::get_workspace_cursor(&conn, &WorkspaceId::from("ws_2")).expect("cursor"),
        CursorSeq(2)
    );
    let task_after = TaskStore::get_task(&conn, &TaskId::from("task_2"))
        .expect("get task")
        .expect("exists");
    assert_eq!(task_after.title, "Synced task");

    let valid_batch = vec![
        task_upsert_envelope(3, "Applied atomically", 230),
        comment_envelope(4, "cmt_ok", "Contiguous comment", 231),
    ];
    let new_cursor = SyncStore::apply_events_batch_atomic(
        &mut conn,
        &WorkspaceId::from("ws_2"),
        "http://127.0.0.1:8080",
        &valid_batch,
        Timestamp(235),
    )
    .expect("apply valid batch");
    assert_eq!(new_cursor, CursorSeq(4));
    assert_eq!(
        CommentStore::list_comments_for_task(&conn, &TaskId::from("task_2"))
            .expect("comments")
            .len(),
        1
    );
}

#[test]
fn test_save_session_does_not_inherit_workspace_or_runner_across_users() {
    let conn = setup_db();
    TeamAuthStore::save_session(
        &conn,
        &TeamAuthSession {
            server_url: "http://127.0.0.1:8080".to_string(),
            user_id: UserId::from("usr_alice"),
            display_name: "Alice".to_string(),
            bearer_token: "tok_alice".to_string(),
            default_workspace_id: Some(WorkspaceId::from("ws_alice")),
            runner_id: Some(RunnerId::from("rnr_alice")),
            runner_name: Some("Alice Laptop".to_string()),
            updated_at: Timestamp(100),
        },
    )
    .expect("save alice");

    // Re-login by the same user preserves default workspace and runner identity.
    TeamAuthStore::save_session(
        &conn,
        &TeamAuthSession {
            server_url: "http://127.0.0.1:8080/".to_string(),
            user_id: UserId::from("usr_alice"),
            display_name: "Alice Updated".to_string(),
            bearer_token: "tok_alice_2".to_string(),
            default_workspace_id: None,
            runner_id: None,
            runner_name: None,
            updated_at: Timestamp(200),
        },
    )
    .expect("resave alice");
    let alice = TeamAuthStore::get_by_server(&conn, "http://127.0.0.1:8080")
        .expect("get alice")
        .expect("exists");
    assert_eq!(
        alice.default_workspace_id,
        Some(WorkspaceId::from("ws_alice"))
    );
    assert_eq!(alice.runner_id, Some(RunnerId::from("rnr_alice")));
    assert_eq!(alice.runner_name.as_deref(), Some("Alice Laptop"));

    // Logging in as a different user on the same server must NOT inherit Alice's workspace or runner.
    TeamAuthStore::save_session(
        &conn,
        &TeamAuthSession {
            server_url: "http://127.0.0.1:8080".to_string(),
            user_id: UserId::from("usr_bob"),
            display_name: "Bob".to_string(),
            bearer_token: "tok_bob".to_string(),
            default_workspace_id: None,
            runner_id: None,
            runner_name: None,
            updated_at: Timestamp(300),
        },
    )
    .expect("save bob");
    let bob = TeamAuthStore::get_by_server(&conn, "http://127.0.0.1:8080")
        .expect("get bob")
        .expect("exists");
    assert_eq!(bob.user_id, UserId::from("usr_bob"));
    assert_eq!(bob.default_workspace_id, None);
    assert_eq!(bob.runner_id, None);
    assert_eq!(bob.runner_name, None);
}
