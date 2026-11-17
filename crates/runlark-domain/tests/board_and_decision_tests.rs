use runlark_domain::{
    ActorRole, ArtifactDigest, BudgetPolicy, Decision, DecisionBinding, DecisionId, DecisionKind,
    DomainError, DomainEvent, PolicyDigest, Project, ProjectId, Revision, Run, RunId, RunStatus,
    SessionId, Task, TaskId, TaskKind, TaskStage, Timestamp, TransitionError, WorkflowStage,
    advance_run_stage, apply_decision, move_task_stage, start_task_run, update_task_plan,
};

fn digest_a() -> ArtifactDigest {
    ArtifactDigest::parse("a".repeat(64)).expect("valid digest")
}

fn digest_b() -> ArtifactDigest {
    ArtifactDigest::parse("b".repeat(64)).expect("valid digest")
}

fn sample_task(kind: TaskKind) -> Task {
    Task {
        id: TaskId::from("task_1"),
        project_id: ProjectId::from("proj_1"),
        title: "Implement A1 domain".to_string(),
        description: "Pure domain rules".to_string(),
        kind,
        stage: TaskStage::Inbox,
        workflow_stage: kind.initial_workflow_stage(),
        plan_text: None,
        plan_version: 1,
        base_ref: Some("main".to_string()),
        head_ref: Some("codex/a1".to_string()),
        pr_url: None,
        active_session_id: None,
        active_run_id: None,
        latest_run_id: None,
        budget_override: None,
        revision: Revision(1),
        created_at: Timestamp(1000),
        updated_at: Timestamp(1000),
    }
}

#[test]
fn test_project_defaults_and_digest_validation() {
    let project = Project::new(
        ProjectId::from("proj_1"),
        "Runlark",
        "/repo/runlark",
        "main",
        BudgetPolicy::default(),
        PolicyDigest::from("policy_v1"),
        Timestamp(1000),
    );
    assert_eq!(project.max_concurrent_runs, 2);
    assert_eq!(project.revision, Revision(1));

    assert!(matches!(
        ArtifactDigest::parse("short"),
        Err(DomainError::InvalidDigest(_))
    ));
    assert!(matches!(
        ArtifactDigest::parse("A".repeat(64)),
        Err(DomainError::InvalidDigest(_))
    ));
    assert!(matches!(
        ArtifactDigest::parse("g".repeat(64)),
        Err(DomainError::InvalidDigest(_))
    ));
    assert!(ArtifactDigest::parse("0123456789abcdef".repeat(4)).is_ok());

    let bad_json = format!("\"{}\"", "Z".repeat(64));
    assert!(serde_json::from_str::<ArtifactDigest>(&bad_json).is_err());
}

#[test]
fn test_quick_fix_vs_standard_workflow_pipeline_and_slot_rules() {
    assert_eq!(
        TaskKind::QuickFix.initial_workflow_stage(),
        WorkflowStage::Implementation
    );
    assert_eq!(
        TaskKind::Standard.initial_workflow_stage(),
        WorkflowStage::Clarification
    );

    assert!(RunStatus::Running.consumes_execution_slot());
    assert!(RunStatus::CancelRequested.consumes_execution_slot());
    assert!(!RunStatus::WaitingForApproval.consumes_execution_slot());
    assert!(!RunStatus::Queued.consumes_execution_slot());
    assert!(!RunStatus::Completed.consumes_execution_slot());

    let mut run = Run {
        id: RunId::from("run_std"),
        session_id: SessionId::from("sess_1"),
        status: RunStatus::Running,
        worktree_path: "/wt/1".to_string(),
        task_kind: TaskKind::Standard,
        workflow_stage: WorkflowStage::Clarification,
        ..Run::default()
    };

    // Standard cannot jump directly from Clarification to Implementation
    assert_eq!(
        advance_run_stage(&run, WorkflowStage::Implementation, Timestamp(1100)).unwrap_err(),
        TransitionError::InvalidWorkflowStageTransition {
            from: WorkflowStage::Clarification,
            to: WorkflowStage::Implementation,
        }
    );

    // Standard pipeline: Clarification -> Plan -> Implementation -> Verification -> HumanAcceptance
    run = advance_run_stage(&run, WorkflowStage::Plan, Timestamp(1100))
        .expect("to plan")
        .new_run;
    run = advance_run_stage(&run, WorkflowStage::Implementation, Timestamp(1200))
        .expect("to impl")
        .new_run;
    run = advance_run_stage(&run, WorkflowStage::Verification, Timestamp(1300))
        .expect("to verify")
        .new_run;
    let acceptance_out = advance_run_stage(&run, WorkflowStage::HumanAcceptance, Timestamp(1400))
        .expect("to human acceptance");
    assert_eq!(
        acceptance_out.new_run.workflow_stage,
        WorkflowStage::HumanAcceptance
    );
    // HumanAcceptance frees execution slot by moving RunStatus to WaitingForApproval
    assert_eq!(acceptance_out.new_run.status, RunStatus::WaitingForApproval);
    assert!(!acceptance_out.new_run.status.consumes_execution_slot());
}

#[test]
fn test_board_stage_guards_prevent_agent_self_acceptance_and_active_run_eviction() {
    let mut task = sample_task(TaskKind::Standard);
    let human = ActorRole::Human("alice".to_string());
    let agent = ActorRole::Agent("dsh-worker".to_string());

    // Inbox -> Ready
    task = move_task_stage(
        &task,
        TaskStage::Ready,
        &human,
        Revision(1),
        false,
        false,
        Timestamp(1100),
    )
    .expect("inbox -> ready")
    .new_task;

    // Start run moves Ready -> InProgress
    task = start_task_run(
        &task,
        &SessionId::from("sess_1"),
        &RunId::from("run_1"),
        false,
        task.revision,
        Timestamp(1200),
    )
    .expect("start run")
    .new_task;
    assert_eq!(task.stage, TaskStage::InProgress);

    // Cannot move InProgress -> Ready while run is executing
    assert_eq!(
        move_task_stage(
            &task,
            TaskStage::Ready,
            &human,
            task.revision,
            true,
            false,
            Timestamp(1250),
        )
        .unwrap_err(),
        TransitionError::TaskHasActiveRun {
            run_id: RunId::from("run_1"),
        }
    );

    // Move InProgress -> Acceptance (e.g. by system/human after verification passes)
    task = move_task_stage(
        &task,
        TaskStage::Acceptance,
        &human,
        task.revision,
        false,
        false,
        Timestamp(1300),
    )
    .expect("in_progress -> acceptance")
    .new_task;

    // Agent CANNOT move task to Done or self-accept
    assert_eq!(
        move_task_stage(
            &task,
            TaskStage::Done,
            &agent,
            task.revision,
            false,
            true,
            Timestamp(1400),
        )
        .unwrap_err(),
        TransitionError::AgentCannotSelfAccept
    );

    // Human CANNOT move Acceptance -> Done without a recorded Decision::Accept
    assert_eq!(
        move_task_stage(
            &task,
            TaskStage::Done,
            &human,
            task.revision,
            false,
            false,
            Timestamp(1450),
        )
        .unwrap_err(),
        TransitionError::AcceptanceDecisionRequired
    );
}

#[test]
fn test_decision_binding_and_acceptance_transition() {
    let mut task = sample_task(TaskKind::QuickFix);
    task = update_task_plan(&task, "Quick fix plan", Revision(1), Timestamp(1050))
        .expect("update plan")
        .new_task;
    assert_eq!(task.plan_version, 2);

    task.stage = TaskStage::Acceptance;
    task.workflow_stage = WorkflowStage::HumanAcceptance;
    task.active_run_id = Some(RunId::from("run_qf"));

    let run = Run {
        id: RunId::from("run_qf"),
        session_id: SessionId::from("sess_qf"),
        task_id: Some(task.id.clone()),
        project_id: Some(task.project_id.clone()),
        status: RunStatus::WaitingForApproval,
        workflow_stage: WorkflowStage::HumanAcceptance,
        task_kind: TaskKind::QuickFix,
        plan_version: 2,
        head_sha: Some("head_111".to_string()),
        base_sha: Some("base_000".to_string()),
        policy_digest: Some(PolicyDigest::from("policy_v1")),
        ..Run::default()
    };

    let policy = PolicyDigest::from("policy_v1");
    let evidence = digest_a();

    let decision = Decision {
        id: DecisionId::from("dec_1"),
        task_id: task.id.clone(),
        run_id: run.id.clone(),
        kind: DecisionKind::Accept,
        actor: "alice".to_string(),
        plan_version: 2,
        head_sha: "head_111".to_string(),
        base_sha: "base_000".to_string(),
        policy_digest: policy.clone(),
        evidence_digest: evidence.clone(),
        comment: Some("Looks good".to_string()),
        created_at: Timestamp(2000),
    };

    // Stale head_sha MUST reject
    let drifted_head = DecisionBinding {
        plan_version: 2,
        head_sha: "head_222",
        base_sha: "base_000",
        policy_digest: &policy,
        evidence_digest: &evidence,
    };
    assert!(matches!(
        apply_decision(&task, &run, &decision, &drifted_head, Timestamp(2000)),
        Err(TransitionError::DecisionBindingMismatch { ref field, .. }) if field == "head_sha"
    ));

    // Stale evidence_digest MUST reject
    let other_evidence = digest_b();
    let drifted_evidence = DecisionBinding {
        plan_version: 2,
        head_sha: "head_111",
        base_sha: "base_000",
        policy_digest: &policy,
        evidence_digest: &other_evidence,
    };
    assert!(matches!(
        apply_decision(&task, &run, &decision, &drifted_evidence, Timestamp(2000)),
        Err(TransitionError::DecisionBindingMismatch { ref field, .. }) if field == "evidence_digest"
    ));

    // Exact binding transitions Acceptance -> Done and WaitingForApproval -> Completed
    let exact = DecisionBinding {
        plan_version: 2,
        head_sha: "head_111",
        base_sha: "base_000",
        policy_digest: &policy,
        evidence_digest: &evidence,
    };
    let outcome =
        apply_decision(&task, &run, &decision, &exact, Timestamp(2000)).expect("accept succeeds");
    assert_eq!(outcome.new_task.stage, TaskStage::Done);
    assert_eq!(outcome.new_task.active_run_id, None);
    assert_eq!(outcome.new_run.status, RunStatus::Completed);
    assert!(outcome.events.iter().any(|e| matches!(
        e,
        DomainEvent::TaskStageChanged {
            from: TaskStage::Acceptance,
            to: TaskStage::Done,
            ..
        }
    )));

    // RequestChanges moves Acceptance -> InProgress & Implementation and re-queues run
    let mut reject_dec = decision;
    reject_dec.id = DecisionId::from("dec_2");
    reject_dec.kind = DecisionKind::RequestChanges;
    reject_dec.comment = Some("Add unit test".to_string());
    let req_changes =
        apply_decision(&task, &run, &reject_dec, &exact, Timestamp(2100)).expect("request changes");
    assert_eq!(req_changes.new_task.stage, TaskStage::InProgress);
    assert_eq!(
        req_changes.new_run.workflow_stage,
        WorkflowStage::Implementation
    );
    assert_eq!(req_changes.new_run.status, RunStatus::Queued);
}
