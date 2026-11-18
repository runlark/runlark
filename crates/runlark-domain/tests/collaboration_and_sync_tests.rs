use runlark_domain::{
    AddCommentCommand, BudgetPolicy, CancelTeamRunCommand, CommandId, CommentId, DraftId,
    DraftKind, DraftSyncState, LeaseEpoch, OfflineDraft, PolicyDigest, Project, ProjectId,
    ProjectOwnershipMode, Revision, Run, RunId, RunOwnership, RunStatus, RunnerId,
    RunnerRegistration, RunnerStatus, SessionId, TaskComment, TaskId, TeamCommand, Timestamp,
    TransitionError, UserId, WorkspaceAction, WorkspaceId, WorkspaceRole,
    authorize_workspace_action, sanitize_project_for_team_snapshot, validate_runner_assignment,
    validate_runner_fencing, validate_solo_to_team_promotion,
};

fn runner(id: &str, status: RunnerStatus) -> RunnerRegistration {
    RunnerRegistration {
        id: RunnerId::from(id),
        workspace_id: WorkspaceId::from("ws_1"),
        owner_user_id: UserId::from("usr_1"),
        name: id.to_string(),
        status,
        last_heartbeat_at: Timestamp(1000),
        created_at: Timestamp(1000),
    }
}

#[test]
fn test_workspace_rbac_matrix() {
    assert_eq!(
        WorkspaceRole::from_str_name("Owner"),
        Some(WorkspaceRole::Owner)
    );
    assert_eq!(WorkspaceRole::from_str_name("invalid"), None);

    assert!(
        authorize_workspace_action(WorkspaceRole::Viewer, WorkspaceAction::ReadWorkspace).is_ok()
    );
    assert!(!WorkspaceRole::Viewer.can_mutate_tasks());
    assert!(matches!(
        authorize_workspace_action(WorkspaceRole::Viewer, WorkspaceAction::MutateTasks),
        Err(TransitionError::WorkspacePermissionDenied { .. })
    ));

    assert!(WorkspaceRole::Member.can_mutate_tasks());
    assert!(WorkspaceRole::Member.can_accept_decisions());
    assert!(WorkspaceRole::Member.can_manage_runners());
    assert!(!WorkspaceRole::Member.can_invite());
    assert!(matches!(
        authorize_workspace_action(WorkspaceRole::Member, WorkspaceAction::InviteMembers),
        Err(TransitionError::WorkspacePermissionDenied { .. })
    ));
    assert!(
        authorize_workspace_action(WorkspaceRole::Member, WorkspaceAction::RecordDecision).is_ok()
    );

    assert!(WorkspaceRole::Maintainer.can_invite());
    assert!(WorkspaceRole::Maintainer.can_manage_members());
    assert!(
        authorize_workspace_action(WorkspaceRole::Maintainer, WorkspaceAction::InviteMembers)
            .is_ok()
    );
    assert!(
        authorize_workspace_action(WorkspaceRole::Maintainer, WorkspaceAction::PromoteProject)
            .is_ok()
    );
    assert!(
        authorize_workspace_action(WorkspaceRole::Owner, WorkspaceAction::ManageMembers).is_ok()
    );
}

#[test]
fn test_runner_assignment_fencing_and_no_auto_takeover() {
    let run_id = RunId::from("run_1");
    let r_a = runner("runner_a", RunnerStatus::Online);
    let r_b = runner("runner_b", RunnerStatus::Online);
    let r_offline = runner("runner_off", RunnerStatus::Unreachable);

    let unassigned = RunOwnership::default();
    assert!(matches!(
        validate_runner_assignment(&run_id, RunStatus::Queued, &unassigned, &r_offline),
        Err(TransitionError::RunnerNotOnline { .. })
    ));
    assert!(matches!(
        validate_runner_assignment(&run_id, RunStatus::Completed, &unassigned, &r_a),
        Err(TransitionError::RunAlreadyTerminated)
    ));

    let epoch_1 = validate_runner_assignment(&run_id, RunStatus::Queued, &unassigned, &r_a)
        .expect("online runner acquires epoch 1");
    assert_eq!(epoch_1, LeaseEpoch(1));

    let owned_by_a = RunOwnership {
        assigned_runner_id: Some(r_a.id.clone()),
        lease_epoch: epoch_1,
        runner_unreachable: true,
    };

    // Re-confirming same online runner keeps existing lease_epoch.
    assert_eq!(
        validate_runner_assignment(&run_id, RunStatus::Running, &owned_by_a, &r_a)
            .expect("same runner keeps epoch"),
        LeaseEpoch(1)
    );

    // Even when runner_a is unreachable, runner_b cannot auto-takeover active run.
    assert!(matches!(
        validate_runner_assignment(&run_id, RunStatus::Running, &owned_by_a, &r_b),
        Err(TransitionError::ActiveRunPreventsTakeover { .. })
    ));

    // Fencing checks reject wrong runner, stale epoch, or higher unowned epoch.
    assert!(validate_runner_fencing(&run_id, &owned_by_a, &r_a.id, LeaseEpoch(1)).is_ok());
    assert!(matches!(
        validate_runner_fencing(&run_id, &owned_by_a, &r_b.id, LeaseEpoch(1)),
        Err(TransitionError::RunnerFencingMismatch { .. })
    ));
    assert!(matches!(
        validate_runner_fencing(&run_id, &owned_by_a, &r_a.id, LeaseEpoch(0)),
        Err(TransitionError::RunnerFencingMismatch { .. })
    ));
    assert!(matches!(
        validate_runner_fencing(&run_id, &owned_by_a, &r_a.id, LeaseEpoch(2)),
        Err(TransitionError::RunnerFencingMismatch { .. })
    ));
}

#[test]
fn test_solo_to_team_promotion_rejects_active_runs_and_strips_local_paths() {
    let terminal_runs =
        [RunStatus::Completed, RunStatus::Failed, RunStatus::Stopped].map(|status| Run {
            id: RunId::from("run_done"),
            session_id: SessionId::from("sess_1"),
            status,
            ..Default::default()
        });
    assert!(validate_solo_to_team_promotion(&terminal_runs).is_ok());

    for blocking_status in [
        RunStatus::Queued,
        RunStatus::Running,
        RunStatus::WaitingForApproval,
        RunStatus::CancelRequested,
        RunStatus::Interrupted,
    ] {
        let active = Run {
            id: RunId::from("run_active"),
            session_id: SessionId::from("sess_2"),
            status: blocking_status,
            ..Default::default()
        };
        assert!(matches!(
            validate_solo_to_team_promotion(&[active]),
            Err(TransitionError::SoloToTeamBlockedByActiveRuns { active_runs: 1 })
        ));
    }

    let project = Project::new(
        ProjectId::from("proj_1"),
        "Runlark",
        "/Users/secret/local/checkout",
        "main",
        BudgetPolicy::default(),
        PolicyDigest::from("pol_1"),
        Timestamp(100),
    );
    let sanitized = sanitize_project_for_team_snapshot(&project, WorkspaceId::from("ws_1"), 1);
    let serialized = serde_json::to_string(&sanitized).expect("serialize sanitized project");
    assert!(!serialized.contains("/Users/secret/local/checkout"));

    let mode = ProjectOwnershipMode::TeamManaged {
        workspace_id: WorkspaceId::from("ws_1"),
        server_url: "https://team.runlark.local".to_string(),
        ownership_epoch: 1,
    };
    let roundtrip: ProjectOwnershipMode =
        serde_json::from_str(&serde_json::to_string(&mode).expect("serialize mode"))
            .expect("deserialize mode");
    assert_eq!(roundtrip, mode);
    assert_eq!(
        ProjectOwnershipMode::default(),
        ProjectOwnershipMode::SoloLocal
    );
}

#[test]
fn test_team_command_canonical_hash_and_offline_draft_roundtrip() {
    let c1 = TeamCommand::AddComment(AddCommentCommand {
        command_id: CommandId::from("cmd_1"),
        comment_id: CommentId::from("cmt_1"),
        task_id: TaskId::from("task_1"),
        run_id: None,
        body: "Looks good".to_string(),
        created_at: Timestamp(100),
    });
    let c2 = TeamCommand::AddComment(AddCommentCommand {
        command_id: CommandId::from("cmd_1"),
        comment_id: CommentId::from("cmt_1"),
        task_id: TaskId::from("task_1"),
        run_id: None,
        body: "Looks good".to_string(),
        created_at: Timestamp(9999),
    });
    assert_eq!(c1.canonical_for_hash(), c2.canonical_for_hash());

    let cancel_1 = TeamCommand::CancelRun(CancelTeamRunCommand {
        command_id: CommandId::from("cmd_cancel"),
        run_id: RunId::from("run_1"),
        reason: "user stopped".to_string(),
        expected_revision: Revision(2),
        created_at: Timestamp(10),
    });
    let cancel_2 = TeamCommand::CancelRun(CancelTeamRunCommand {
        command_id: CommandId::from("cmd_cancel"),
        run_id: RunId::from("run_1"),
        reason: "user stopped".to_string(),
        expected_revision: Revision(2),
        created_at: Timestamp(500),
    });
    assert_eq!(cancel_1.canonical_for_hash(), cancel_2.canonical_for_hash());
    assert_eq!(cancel_1.command_id(), &CommandId::from("cmd_cancel"));

    let draft = OfflineDraft {
        id: DraftId::from("draft_1"),
        project_id: ProjectId::from("proj_1"),
        task_id: Some(TaskId::from("task_1")),
        kind: DraftKind::TaskComment,
        payload_json: "{\"body\":\"offline\"}".to_string(),
        base_revision: Some(Revision(3)),
        sync_state: DraftSyncState::Conflict {
            server_revision: Some(Revision(4)),
            reason: "revision mismatch".to_string(),
        },
        created_at: Timestamp(100),
        updated_at: Timestamp(200),
    };
    let parsed_draft: OfflineDraft =
        serde_json::from_str(&serde_json::to_string(&draft).expect("serialize draft"))
            .expect("deserialize draft");
    assert_eq!(parsed_draft, draft);

    let comment = TaskComment {
        id: CommentId::from("cmt_1"),
        task_id: TaskId::from("task_1"),
        run_id: None,
        author_user_id: UserId::from("usr_1"),
        author_display_name: "Alice".to_string(),
        body: "Append-only".to_string(),
        created_at: Timestamp(123),
    };
    assert_eq!(comment.body, "Append-only");
}
