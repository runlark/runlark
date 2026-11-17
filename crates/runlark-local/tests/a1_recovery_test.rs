use runlark_domain::{
    ArtifactKind, Attempt, AttemptId, AttemptOutcome, BoardCommand, BudgetPolicy, CommandId,
    CreateProjectCommand, CreateTaskCommand, DecisionId, DecisionKind, PolicyDigest, ProjectId,
    Revision, Run, RunId, RunStatus, Session, SessionId, SessionStatus, TaskId, TaskKind,
    TaskStage, Timestamp, WorkflowStage,
};
use runlark_local::{
    ArtifactCasStore, AttemptStore, BoardCommandOutcome, BoardProjection, BoardStore,
    DecisionStore, ProjectStore, ReconciliationEngine, SessionStore, TaskStore, initialize_schema,
    open_sqlite,
};
use tempfile::TempDir;

fn seed_recovery_artifact_and_decision(
    conn: &runlark_local::Connection,
    cas: &ArtifactCasStore,
) -> runlark_domain::ArtifactDigest {
    let art = cas
        .put_bytes(
            conn,
            ArtifactKind::Plan,
            "text/markdown",
            b"# Recovery plan\n",
            Timestamp(1150),
        )
        .unwrap();
    ArtifactCasStore::link_to_run(
        conn,
        &RunId::from("r_wait"),
        &art.digest_sha256,
        "plan",
        Timestamp(1150),
    )
    .unwrap();

    DecisionStore::record_decision(
        conn,
        &runlark_domain::Decision {
            id: DecisionId::from("dec_plan_1"),
            task_id: TaskId::from("task_waiting"),
            run_id: RunId::from("r_wait"),
            kind: DecisionKind::ApprovePlan,
            actor: "alice".to_string(),
            plan_version: 1,
            head_sha: "head_0".to_string(),
            base_sha: "base_0".to_string(),
            policy_digest: PolicyDigest::from("pol_rec"),
            evidence_digest: art.digest_sha256.clone(),
            comment: None,
            created_at: Timestamp(1160),
        },
    )
    .unwrap();

    art.digest_sha256
}

fn seed_recovery_state(
    db_path: &std::path::Path,
    cas: &ArtifactCasStore,
) -> runlark_domain::ArtifactDigest {
    let mut conn = open_sqlite(db_path).expect("open db");
    initialize_schema(&conn).expect("init schema");

    let proj_cmd = BoardCommand::CreateProject(CreateProjectCommand {
        command_id: CommandId::from("cmd_p1"),
        project_id: ProjectId::from("proj_rec"),
        name: "RecoveryRepo".to_string(),
        repo_root: "/repo".to_string(),
        default_branch: "main".to_string(),
        budget_policy: BudgetPolicy::default(),
        policy_digest: PolicyDigest::from("pol_rec"),
        created_at: Timestamp(1000),
    });
    assert!(matches!(
        BoardStore::apply_board_command(&mut conn, &proj_cmd, Timestamp(1000)).unwrap(),
        BoardCommandOutcome::Applied { .. }
    ));

    for tid in ["task_running", "task_waiting", "task_q1", "task_q2"] {
        let cmd = BoardCommand::CreateTask(CreateTaskCommand {
            command_id: CommandId::from(format!("cmd_{tid}")),
            task_id: TaskId::from(tid),
            project_id: ProjectId::from("proj_rec"),
            title: format!("Task {tid}"),
            description: "Recovery test".to_string(),
            kind: TaskKind::Standard,
            base_ref: Some("main".to_string()),
            budget_override: None,
            created_at: Timestamp(1010),
        });
        BoardStore::apply_board_command(&mut conn, &cmd, Timestamp(1010)).unwrap();
    }

    for (sid, tid, rid, status, ts) in [
        ("s_run", "task_running", "r_run", RunStatus::Running, 1100),
        (
            "s_wait",
            "task_waiting",
            "r_wait",
            RunStatus::WaitingForApproval,
            1110,
        ),
        ("s_q1", "task_q1", "r_q1", RunStatus::Queued, 1120),
        ("s_q2", "task_q2", "r_q2", RunStatus::Queued, 1130),
    ] {
        SessionStore::create_session(
            &conn,
            &Session {
                id: SessionId::from(sid),
                task_id: TaskId::from(tid),
                cwd: "/repo".to_string(),
                status: SessionStatus::Active,
                current_revision: Revision(1),
                active_run_id: Some(RunId::from(rid)),
                created_at: Timestamp(ts),
                updated_at: Timestamp(ts),
            },
        )
        .unwrap();
        SessionStore::save_run(
            &conn,
            &Run {
                id: RunId::from(rid),
                session_id: SessionId::from(sid),
                task_id: Some(TaskId::from(tid)),
                project_id: Some(ProjectId::from("proj_rec")),
                status,
                workflow_stage: WorkflowStage::Implementation,
                worktree_path: format!("/wt/{rid}"),
                created_at: Timestamp(ts),
                updated_at: Timestamp(ts),
                ..Run::default()
            },
        )
        .unwrap();
    }

    AttemptStore::save_attempt(
        &conn,
        &Attempt {
            id: AttemptId::from("att_inflight"),
            run_id: RunId::from("r_run"),
            attempt_no: 1,
            workflow_stage: WorkflowStage::Implementation,
            started_at: Timestamp(1105),
            ended_at: None,
            outcome: AttemptOutcome::InProgress,
            cost_micro_usd: None,
            unknown_cost_reason: None,
            repair_reason: None,
        },
    )
    .unwrap();

    seed_recovery_artifact_and_decision(&conn, cas)
}

#[test]
fn test_a1_restart_recovery_preserves_board_cas_unknown_costs_and_queued_slots() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("runlark.db");
    let cas = ArtifactCasStore::from_data_dir(dir.path());
    let artifact_digest = seed_recovery_state(&db_path, &cas);

    let mut conn = open_sqlite(&db_path).expect("reopen db");
    let inflight =
        ReconciliationEngine::reconcile_on_startup(&mut conn, Timestamp(5000)).expect("reconcile");

    assert_eq!(inflight.active_runs.len(), 1);
    assert_eq!(inflight.active_runs[0].0, RunId::from("r_run"));
    assert_eq!(inflight.interrupted_attempts, 1);
    assert_eq!(inflight.waiting_runs.len(), 1);
    assert_eq!(inflight.waiting_runs[0].0, RunId::from("r_wait"));
    assert_eq!(
        inflight
            .queued_runs
            .iter()
            .map(|(r, _, _)| r.as_ref())
            .collect::<Vec<_>>(),
        vec!["r_q1", "r_q2"]
    );

    assert!(
        ProjectStore::get_project(&conn, &ProjectId::from("proj_rec"))
            .unwrap()
            .is_some()
    );
    assert_eq!(
        TaskStore::get_task(&conn, &TaskId::from("task_q1"))
            .unwrap()
            .unwrap()
            .stage,
        TaskStage::Inbox
    );
    assert_eq!(
        DecisionStore::list_decisions_for_run(&conn, &RunId::from("r_wait"))
            .unwrap()
            .len(),
        1
    );
    assert!(
        ReconciliationEngine::verify_cas_integrity(&conn, &cas)
            .unwrap()
            .is_empty()
    );

    let run_after = SessionStore::get_run(&conn, &RunId::from("r_run"))
        .unwrap()
        .unwrap();
    assert_eq!(run_after.status, RunStatus::Interrupted);
    assert_eq!(run_after.cost_summary.unknown_attempts, 1);
    assert_eq!(run_after.cost_summary.known_micro_usd, 0);

    let att_after = AttemptStore::get_attempt(&conn, &AttemptId::from("att_inflight"))
        .unwrap()
        .unwrap();
    assert!(matches!(att_after.outcome, AttemptOutcome::Unknown { .. }));
    assert_eq!(att_after.cost_micro_usd, None);

    assert_eq!(BoardProjection::count_executing_runs(&conn).unwrap(), 0);
    let queued = BoardProjection::list_queued_runs_fifo(&conn).unwrap();
    assert_eq!(queued.len(), 2);
    assert_eq!(queued[0].id, RunId::from("r_q1"));
    assert_eq!(queued[1].id, RunId::from("r_q2"));

    std::fs::remove_file(cas.blob_path(&artifact_digest)).unwrap();
    let bad = ReconciliationEngine::verify_cas_integrity(&conn, &cas).unwrap();
    assert_eq!(bad, vec![artifact_digest]);
}
