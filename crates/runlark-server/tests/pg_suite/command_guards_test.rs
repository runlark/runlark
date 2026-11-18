use super::harness::{
    TestServer, accept_invite, bootstrap_user, create_invite, create_ws, get_snapshot,
    post_command, promote_project, sample_run_cmd, sample_task_cmd, spawn_test_server,
};
use reqwest::StatusCode;
use runlark_domain::{
    ActorRole, ArtifactDigest, ArtifactKind, AttachArtifactCommand, AttemptId, AttemptOutcome,
    BudgetPolicy, CancelTeamRunCommand, CommandId, DecisionId, DecisionKind, LeaseEpoch,
    MoveTaskStageCommand, PolicyDigest, ProjectId, RecordAttemptCommand, RecordDecisionCommand,
    Revision, Run, RunId, RunStatus, RunnerId, RunnerRunReport, SessionId, Task, TaskId, TaskKind,
    TaskStage, TeamCommand, TeamProjectSpec, Timestamp, WorkflowStage, WorkspaceId,
};
use runlark_server::cas::sha256_hex;
use serde_json::json;

#[tokio::test]
async fn test_create_team_run_rejects_done_task() {
    let srv = spawn_test_server().await;
    let tok = bootstrap_user(&srv, "usr_done_guard", "Owner").await;
    create_ws(&srv, &tok, "ws_done_guard", "ws-done-guard").await;

    let project_spec = TeamProjectSpec {
        project_id: ProjectId::from("proj_done_guard"),
        workspace_id: WorkspaceId::from("ws_done_guard"),
        name: "Repo".to_string(),
        default_branch: "main".to_string(),
        max_concurrent_runs: 2,
        budget_policy: BudgetPolicy::default(),
        policy_digest: PolicyDigest::from("pol_v1"),
        ownership_epoch: 1,
        revision: Revision(1),
        created_at: Timestamp(100),
        updated_at: Timestamp(100),
    };
    let done_task = Task {
        id: TaskId::from("task_done_1"),
        project_id: ProjectId::from("proj_done_guard"),
        title: "Already done".to_string(),
        description: String::new(),
        kind: TaskKind::QuickFix,
        stage: TaskStage::Done,
        workflow_stage: WorkflowStage::HumanAcceptance,
        plan_text: None,
        plan_version: 1,
        base_ref: Some("main".to_string()),
        head_ref: None,
        pr_url: None,
        active_session_id: None,
        active_run_id: None,
        latest_run_id: Some(RunId::from("run_done_0")),
        budget_override: None,
        revision: Revision(3),
        created_at: Timestamp(100),
        updated_at: Timestamp(200),
    };
    let done_run = Run {
        id: RunId::from("run_done_0"),
        session_id: SessionId::from("sess_done_0"),
        status: RunStatus::Completed,
        worktree_path: String::new(),
        project_id: Some(ProjectId::from("proj_done_guard")),
        task_id: Some(TaskId::from("task_done_1")),
        ..Default::default()
    };

    let promo = srv
        .http
        .post(format!(
            "{}/api/v1/workspaces/ws_done_guard/projects/promote",
            srv.base_url
        ))
        .bearer_auth(&tok)
        .json(&json!({
            "project": project_spec,
            "tasks": [done_task],
            "runs": [done_run],
            "decisions": []
        }))
        .send()
        .await
        .expect("promote done task");
    assert_eq!(promo.status(), StatusCode::OK);

    let cmd = TeamCommand::CreateTeamRun(runlark_domain::CreateTeamRunCommand {
        command_id: CommandId::from("cmd_done_run"),
        run_id: RunId::from("run_illegal"),
        project_id: ProjectId::from("proj_done_guard"),
        task_id: TaskId::from("task_done_1"),
        runner_id: None,
        expected_revision: Revision(3),
        created_at: Timestamp(300),
    });
    assert_eq!(
        post_command(&srv, &tok, "ws_done_guard", &cmd)
            .await
            .status(),
        StatusCode::CONFLICT
    );

    let snap = get_snapshot(&srv, &tok, "ws_done_guard").await;
    assert_eq!(snap.tasks[0].stage, TaskStage::Done);
    assert_eq!(snap.tasks[0].active_run_id, None);
    assert_eq!(snap.runs.len(), 1);
}

#[tokio::test]
async fn test_record_attempt_rejects_terminal_run() {
    let srv = spawn_test_server().await;
    let tok = bootstrap_user(&srv, "usr_att_guard", "Owner").await;
    create_ws(&srv, &tok, "ws_att_guard", "ws-att-guard").await;
    promote_project(&srv, &tok, "ws_att_guard", "proj_att_guard").await;

    assert_eq!(
        post_command(
            &srv,
            &tok,
            "ws_att_guard",
            &sample_task_cmd("cmd_ag_t1", "task_ag_1", "proj_att_guard", "Attempt guard")
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        post_command(
            &srv,
            &tok,
            "ws_att_guard",
            &sample_run_cmd("cmd_ag_r1", "run_ag_1", "proj_att_guard", "task_ag_1", None)
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    let cancel_q = TeamCommand::CancelRun(CancelTeamRunCommand {
        command_id: CommandId::from("cmd_ag_cancel"),
        run_id: RunId::from("run_ag_1"),
        reason: "stop".to_string(),
        expected_revision: Revision(1),
        created_at: Timestamp(200),
    });
    assert_eq!(
        post_command(&srv, &tok, "ws_att_guard", &cancel_q)
            .await
            .status(),
        StatusCode::CREATED
    );

    let att = TeamCommand::RecordAttempt(RecordAttemptCommand {
        command_id: CommandId::from("cmd_ag_att"),
        attempt_id: AttemptId::from("att_illegal"),
        run_id: RunId::from("run_ag_1"),
        workflow_stage: WorkflowStage::Implementation,
        outcome: AttemptOutcome::Succeeded,
        cost_micro_usd: Some(500),
        unknown_cost_reason: None,
        repair_reason: None,
        expected_revision: Revision(2),
        created_at: Timestamp(250),
    });
    assert_eq!(
        post_command(&srv, &tok, "ws_att_guard", &att)
            .await
            .status(),
        StatusCode::CONFLICT
    );

    let snap = get_snapshot(&srv, &tok, "ws_att_guard").await;
    assert!(snap.attempts.is_empty());
    assert_eq!(snap.runs[0].run.status, RunStatus::Stopped);
    assert_eq!(snap.runs[0].run.revision, Revision(2));
}

async fn setup_acceptance_task_and_run(
    srv: &TestServer,
    tok: &str,
    suffix: &str,
    tid: &str,
    rid: &str,
    digest: &str,
    payload_len: u64,
) {
    assert_eq!(
        post_command(
            srv,
            tok,
            "ws_dec_guard",
            &sample_task_cmd(
                &format!("cmd_dg_t_{suffix}"),
                tid,
                "proj_dec_guard",
                "Guard task"
            )
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        post_command(
            srv,
            tok,
            "ws_dec_guard",
            &sample_run_cmd(
                &format!("cmd_dg_r_{suffix}"),
                rid,
                "proj_dec_guard",
                tid,
                Some("runner_dg")
            )
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    let mv = TeamCommand::MoveTaskStage(MoveTaskStageCommand {
        command_id: CommandId::from(format!("cmd_dg_mv_{suffix}")),
        task_id: TaskId::from(tid),
        target_stage: TaskStage::Acceptance,
        actor: ActorRole::Human("Owner".to_string()),
        expected_revision: Revision(2),
        created_at: Timestamp(120),
    });
    assert_eq!(
        post_command(srv, tok, "ws_dec_guard", &mv).await.status(),
        StatusCode::CREATED
    );

    for stage in [WorkflowStage::Verification, WorkflowStage::HumanAcceptance] {
        let res = srv
            .http
            .post(format!(
                "{}/api/v1/workspaces/ws_dec_guard/runs/{rid}/runner-state",
                srv.base_url
            ))
            .bearer_auth(tok)
            .json(&RunnerRunReport {
                runner_id: RunnerId::from("runner_dg"),
                lease_epoch: LeaseEpoch(1),
                status: None,
                workflow_stage: Some(stage),
                head_sha: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
                base_sha: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
                events: vec![],
            })
            .send()
            .await
            .expect("advance stage");
        assert_eq!(res.status(), StatusCode::OK);
    }

    let attach = TeamCommand::AttachArtifact(AttachArtifactCommand {
        command_id: CommandId::from(format!("cmd_dg_att_{suffix}")),
        run_id: RunId::from(rid),
        digest_sha256: ArtifactDigest::parse(digest).expect("digest"),
        kind: ArtifactKind::EvidenceBundle,
        media_type: "application/json".to_string(),
        size_bytes: payload_len,
        label: "evidence_bundle".to_string(),
        created_at: Timestamp(140),
    });
    assert_eq!(
        post_command(srv, tok, "ws_dec_guard", &attach)
            .await
            .status(),
        StatusCode::CREATED
    );
}

async fn settle_run_status(srv: &TestServer, tok: &str, wid: &str, rid: &str, status: RunStatus) {
    let res = srv
        .http
        .post(format!(
            "{}/api/v1/workspaces/{wid}/runs/{rid}/runner-state",
            srv.base_url
        ))
        .bearer_auth(tok)
        .json(&RunnerRunReport {
            runner_id: RunnerId::from("runner_dg"),
            lease_epoch: LeaseEpoch(1),
            status: Some(status),
            workflow_stage: None,
            head_sha: None,
            base_sha: None,
            events: vec![],
        })
        .send()
        .await
        .expect("settle run status");
    assert_eq!(res.status(), StatusCode::OK);
}

fn accept_cmd(
    cmd_id: &str,
    dec_id: &str,
    tid: &str,
    rid: &str,
    digest: &str,
    expected_revision: u64,
    ts: u64,
) -> TeamCommand {
    TeamCommand::RecordDecision(RecordDecisionCommand {
        command_id: CommandId::from(cmd_id),
        decision_id: DecisionId::from(dec_id),
        task_id: TaskId::from(tid),
        run_id: RunId::from(rid),
        kind: DecisionKind::Accept,
        actor: "Owner".to_string(),
        plan_version: 1,
        head_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        base_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        policy_digest: PolicyDigest::from("pol_v1"),
        evidence_digest: ArtifactDigest::parse(digest).expect("digest"),
        comment: Some("Should fail on non-active run".to_string()),
        expected_revision: Revision(expected_revision),
        created_at: Timestamp(ts),
    })
}

async fn setup_decision_guard_env() -> (TestServer, String, String, u64) {
    let srv = spawn_test_server().await;
    let tok = bootstrap_user(&srv, "usr_dec_guard", "Owner").await;
    create_ws(&srv, &tok, "ws_dec_guard", "ws-dec-guard").await;
    promote_project(&srv, &tok, "ws_dec_guard", "proj_dec_guard").await;

    let reg = srv
        .http
        .post(format!(
            "{}/api/v1/workspaces/ws_dec_guard/runners/register",
            srv.base_url
        ))
        .bearer_auth(&tok)
        .json(&json!({ "runner_id": "runner_dg", "name": "DG Runner" }))
        .send()
        .await
        .expect("register runner");
    assert_eq!(reg.status(), StatusCode::OK);

    let payload = b"decision-guard-evidence";
    let digest = sha256_hex(payload);
    assert_eq!(
        srv.http
            .put(format!(
                "{}/api/v1/workspaces/ws_dec_guard/artifacts/{digest}?kind=evidence_bundle",
                srv.base_url
            ))
            .bearer_auth(&tok)
            .body(payload.to_vec())
            .send()
            .await
            .expect("upload cas")
            .status(),
        StatusCode::CREATED
    );
    (srv, tok, digest, payload.len() as u64)
}

#[tokio::test]
async fn test_record_decision_rejects_terminal_and_cancelling_runs() {
    let (srv, tok, digest, payload_len) = setup_decision_guard_env().await;

    // Case 1: CancelRequested and Stopped runs in HumanAcceptance MUST reject Accept.
    setup_acceptance_task_and_run(
        &srv,
        &tok,
        "1",
        "task_dg_1",
        "run_dg_1",
        &digest,
        payload_len,
    )
    .await;
    let cancel = TeamCommand::CancelRun(CancelTeamRunCommand {
        command_id: CommandId::from("cmd_dg_cancel_1"),
        run_id: RunId::from("run_dg_1"),
        reason: "cancel".to_string(),
        expected_revision: Revision(4),
        created_at: Timestamp(150),
    });
    assert_eq!(
        post_command(&srv, &tok, "ws_dec_guard", &cancel)
            .await
            .status(),
        StatusCode::CREATED
    );
    let cancel_acc = accept_cmd(
        "cmd_dg_acc_cancelling",
        "dec_dg_cancelling",
        "task_dg_1",
        "run_dg_1",
        &digest,
        3,
        160,
    );
    assert_eq!(
        post_command(&srv, &tok, "ws_dec_guard", &cancel_acc)
            .await
            .status(),
        StatusCode::CONFLICT
    );

    settle_run_status(&srv, &tok, "ws_dec_guard", "run_dg_1", RunStatus::Stopped).await;
    let stop_acc = accept_cmd(
        "cmd_dg_acc_stopped",
        "dec_dg_stopped",
        "task_dg_1",
        "run_dg_1",
        &digest,
        4,
        170,
    );
    assert_eq!(
        post_command(&srv, &tok, "ws_dec_guard", &stop_acc)
            .await
            .status(),
        StatusCode::CONFLICT
    );

    // Case 2: Failed run in HumanAcceptance MUST reject Accept.
    setup_acceptance_task_and_run(
        &srv,
        &tok,
        "2",
        "task_dg_2",
        "run_dg_2",
        &digest,
        payload_len,
    )
    .await;
    settle_run_status(&srv, &tok, "ws_dec_guard", "run_dg_2", RunStatus::Failed).await;
    let fail_acc = accept_cmd(
        "cmd_dg_acc_failed",
        "dec_dg_failed",
        "task_dg_2",
        "run_dg_2",
        &digest,
        4,
        180,
    );
    assert_eq!(
        post_command(&srv, &tok, "ws_dec_guard", &fail_acc)
            .await
            .status(),
        StatusCode::CONFLICT
    );

    let snap = get_snapshot(&srv, &tok, "ws_dec_guard").await;
    assert!(snap.decisions.is_empty());
    let statuses: Vec<(RunId, RunStatus)> = snap
        .runs
        .iter()
        .map(|r| (r.run.id.clone(), r.run.status))
        .collect();
    assert!(statuses.contains(&(RunId::from("run_dg_1"), RunStatus::Stopped)));
    assert!(statuses.contains(&(RunId::from("run_dg_2"), RunStatus::Failed)));
}

#[tokio::test]
async fn test_concurrent_role_revocation_serializes_with_mutating_handlers() {
    let srv = spawn_test_server().await;
    let owner_tok = bootstrap_user(&srv, "usr_race_own", "Owner").await;
    create_ws(&srv, &owner_tok, "ws_race", "ws-race").await;
    promote_project(&srv, &owner_tok, "ws_race", "proj_race").await;

    let (_, inv_tok) = create_invite(&srv, &owner_tok, "ws_race", "m@ex.com", "maintainer").await;
    let actor_tok = accept_invite(&srv, &inv_tok, "usr_race_actor", "Actor").await;

    let demote_fut = srv
        .http
        .patch(format!(
            "{}/api/v1/workspaces/ws_race/members/usr_race_actor",
            srv.base_url
        ))
        .bearer_auth(&owner_tok)
        .json(&json!({ "role": runlark_domain::WorkspaceRole::Viewer }))
        .send();
    let race_cmd = sample_task_cmd("cmd_race_t1", "task_race_1", "proj_race", "Race task");
    let cmd_fut = post_command(&srv, &actor_tok, "ws_race", &race_cmd);

    let (demote_res, cmd_res) = tokio::join!(demote_fut, cmd_fut);
    assert_eq!(demote_res.expect("demote").status(), StatusCode::OK);
    let cmd_status = cmd_res.status();
    assert!(
        cmd_status == StatusCode::CREATED || cmd_status == StatusCode::FORBIDDEN,
        "unexpected concurrent status: {cmd_status}"
    );

    let snap = get_snapshot(&srv, &owner_tok, "ws_race").await;
    if cmd_status == StatusCode::FORBIDDEN {
        assert!(snap.tasks.is_empty());
    } else {
        assert_eq!(snap.tasks.len(), 1);
    }

    // Once revoked to Viewer, all mutating endpoints MUST reject with 403 Forbidden.
    assert_eq!(
        post_command(
            &srv,
            &actor_tok,
            "ws_race",
            &sample_task_cmd("cmd_race_t2", "task_race_2", "proj_race", "After revoke")
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    let reg_after = srv
        .http
        .post(format!(
            "{}/api/v1/workspaces/ws_race/runners/register",
            srv.base_url
        ))
        .bearer_auth(&actor_tok)
        .json(&json!({ "runner_id": "runner_revoked", "name": "Revoked" }))
        .send()
        .await
        .expect("reg after");
    assert_eq!(reg_after.status(), StatusCode::FORBIDDEN);
}
