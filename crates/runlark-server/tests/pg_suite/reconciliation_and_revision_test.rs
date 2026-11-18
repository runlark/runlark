use super::harness::{
    accept_invite, bootstrap_user, create_invite, create_ws, get_snapshot, post_command,
    promote_project, sample_run_cmd, sample_task_cmd, spawn_test_server,
    spawn_test_server_with_timeout,
};
use reqwest::StatusCode;
use runlark_domain::{
    ActorRole, ArtifactDigest, ArtifactKind, AttachArtifactCommand, CancelTeamRunCommand,
    CommandId, DecisionId, DecisionKind, MoveTaskStageCommand, PolicyDigest, ProjectId,
    RecordDecisionCommand, Revision, RunId, RunStatus, RunnerId, RunnerRunReport, RunnerStatus,
    TaskId, TaskStage, TeamCommand, Timestamp, WorkflowStage,
};
use runlark_server::cas::sha256_hex;
use runlark_server::pg::{create_pool, runners};
use serde_json::{Value, json};
use std::time::Duration;

#[tokio::test]
async fn test_expected_revision_conflict_returns_409_and_current_revision() {
    let srv = spawn_test_server().await;
    let tok = bootstrap_user(&srv, "usr_rev", "Rev User").await;
    create_ws(&srv, &tok, "ws_rev", "ws-rev").await;
    promote_project(&srv, &tok, "ws_rev", "proj_rev").await;

    assert_eq!(
        post_command(
            &srv,
            &tok,
            "ws_rev",
            &sample_task_cmd("cmd_rev_t1", "task_rev_1", "proj_rev", "Revision test")
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    let mv_ok = TeamCommand::MoveTaskStage(MoveTaskStageCommand {
        command_id: CommandId::from("cmd_rev_mv1"),
        task_id: TaskId::from("task_rev_1"),
        target_stage: TaskStage::Ready,
        actor: ActorRole::Human("Rev User".to_string()),
        expected_revision: Revision(1),
        created_at: Timestamp(110),
    });
    assert_eq!(
        post_command(&srv, &tok, "ws_rev", &mv_ok).await.status(),
        StatusCode::CREATED
    );

    // Task is now at revision 2; submitting expected_revision = 1 MUST return 409 revision_mismatch.
    let mv_stale = TeamCommand::MoveTaskStage(MoveTaskStageCommand {
        command_id: CommandId::from("cmd_rev_mv_stale"),
        task_id: TaskId::from("task_rev_1"),
        target_stage: TaskStage::InProgress,
        actor: ActorRole::Human("Rev User".to_string()),
        expected_revision: Revision(1),
        created_at: Timestamp(120),
    });
    let task_conflict = post_command(&srv, &tok, "ws_rev", &mv_stale).await;
    assert_eq!(task_conflict.status(), StatusCode::CONFLICT);
    let task_err: Value = task_conflict.json().await.expect("task conflict json");
    assert_eq!(task_err["error"].as_str(), Some("revision_mismatch"));
    assert_eq!(task_err["current_revision"].as_u64(), Some(2));

    // Create a run (revision 1), then submit CancelRun with stale expected_revision = 99.
    let run_cmd = TeamCommand::CreateTeamRun(runlark_domain::CreateTeamRunCommand {
        command_id: CommandId::from("cmd_rev_r1"),
        run_id: RunId::from("run_rev_1"),
        project_id: ProjectId::from("proj_rev"),
        task_id: TaskId::from("task_rev_1"),
        runner_id: None,
        expected_revision: Revision(2),
        created_at: Timestamp(130),
    });
    assert_eq!(
        post_command(&srv, &tok, "ws_rev", &run_cmd).await.status(),
        StatusCode::CREATED
    );
    let cancel_stale = TeamCommand::CancelRun(CancelTeamRunCommand {
        command_id: CommandId::from("cmd_rev_cancel_stale"),
        run_id: RunId::from("run_rev_1"),
        reason: "stale".to_string(),
        expected_revision: Revision(99),
        created_at: Timestamp(140),
    });
    let run_conflict = post_command(&srv, &tok, "ws_rev", &cancel_stale).await;
    assert_eq!(run_conflict.status(), StatusCode::CONFLICT);
    let run_err: Value = run_conflict.json().await.expect("run conflict json");
    assert_eq!(run_err["error"].as_str(), Some("revision_mismatch"));
    assert_eq!(run_err["current_revision"].as_u64(), Some(1));
}

async fn register_runner_ok(srv: &super::harness::TestServer, tok: &str, wid: &str, rid: &str) {
    let reg = srv
        .http
        .post(format!(
            "{}/api/v1/workspaces/{wid}/runners/register",
            srv.base_url
        ))
        .bearer_auth(tok)
        .json(&json!({ "runner_id": rid, "name": rid }))
        .send()
        .await
        .expect("register");
    assert_eq!(reg.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_runner_report_human_acceptance_cannot_overwrite_waiting_for_approval() {
    let srv = spawn_test_server().await;
    let tok = bootstrap_user(&srv, "usr_stage", "Runner Owner").await;
    create_ws(&srv, &tok, "ws_stage", "ws-stage").await;
    promote_project(&srv, &tok, "ws_stage", "proj_stage").await;
    register_runner_ok(&srv, &tok, "ws_stage", "runner_stage").await;

    assert_eq!(
        post_command(
            &srv,
            &tok,
            "ws_stage",
            &sample_task_cmd("cmd_st_t1", "task_st_1", "proj_stage", "Stage guard")
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        post_command(
            &srv,
            &tok,
            "ws_stage",
            &sample_run_cmd(
                "cmd_st_r1",
                "run_st_1",
                "proj_stage",
                "task_st_1",
                Some("runner_stage")
            )
        )
        .await
        .status(),
        StatusCode::CREATED
    );

    let report_url = format!(
        "{}/api/v1/workspaces/ws_stage/runs/run_st_1/runner-state",
        srv.base_url
    );
    let to_verify = RunnerRunReport {
        runner_id: RunnerId::from("runner_stage"),
        lease_epoch: 1,
        status: None,
        workflow_stage: Some(WorkflowStage::Verification),
        head_sha: None,
        base_sha: None,
        events: vec![],
    };
    assert_eq!(
        srv.http
            .post(&report_url)
            .bearer_auth(&tok)
            .json(&to_verify)
            .send()
            .await
            .expect("verify stage")
            .status(),
        StatusCode::OK
    );

    // Combining workflow_stage: HumanAcceptance with status: Running MUST return 409 Conflict.
    let conflicting = RunnerRunReport {
        runner_id: RunnerId::from("runner_stage"),
        lease_epoch: 1,
        status: Some(RunStatus::Running),
        workflow_stage: Some(WorkflowStage::HumanAcceptance),
        head_sha: None,
        base_sha: None,
        events: vec![],
    };
    assert_eq!(
        srv.http
            .post(&report_url)
            .bearer_auth(&tok)
            .json(&conflicting)
            .send()
            .await
            .expect("conflicting stage+status")
            .status(),
        StatusCode::CONFLICT
    );

    let valid_ha = RunnerRunReport {
        status: None,
        ..conflicting
    };
    let ok_res = srv
        .http
        .post(&report_url)
        .bearer_auth(&tok)
        .json(&valid_ha)
        .send()
        .await
        .expect("valid human acceptance");
    assert_eq!(ok_res.status(), StatusCode::OK);
    let snap = get_snapshot(&srv, &tok, "ws_stage").await;
    assert_eq!(
        snap.runs[0].run.workflow_stage,
        WorkflowStage::HumanAcceptance
    );
    assert_eq!(snap.runs[0].run.status, RunStatus::WaitingForApproval);
}

#[tokio::test]
async fn test_heartbeat_and_background_sweeper_mark_stale_runners_unreachable() {
    let srv = spawn_test_server_with_timeout(15).await;
    let owner_tok = bootstrap_user(&srv, "usr_hb_a", "Alice").await;
    create_ws(&srv, &owner_tok, "ws_hb_auto", "ws-hb-auto").await;
    promote_project(&srv, &owner_tok, "ws_hb_auto", "proj_hb_auto").await;

    let (_, inv_tok) = create_invite(&srv, &owner_tok, "ws_hb_auto", "b@ex.com", "member").await;
    let bob_tok = accept_invite(&srv, &inv_tok, "usr_hb_b", "Bob").await;

    for (tok, rid) in [(&owner_tok, "runner_hb_a"), (&bob_tok, "runner_hb_b")] {
        let reg = srv
            .http
            .post(format!(
                "{}/api/v1/workspaces/ws_hb_auto/runners/register",
                srv.base_url
            ))
            .bearer_auth(tok)
            .json(&json!({ "runner_id": rid, "name": rid }))
            .send()
            .await
            .expect("register");
        assert_eq!(reg.status(), StatusCode::OK);
    }

    assert_eq!(
        post_command(
            &srv,
            &owner_tok,
            "ws_hb_auto",
            &sample_task_cmd("cmd_hb_t1", "task_hb_1", "proj_hb_auto", "HB sweep")
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        post_command(
            &srv,
            &owner_tok,
            "ws_hb_auto",
            &sample_run_cmd(
                "cmd_hb_r1",
                "run_hb_1",
                "proj_hb_auto",
                "task_hb_1",
                Some("runner_hb_b")
            )
        )
        .await
        .status(),
        StatusCode::CREATED
    );

    tokio::time::sleep(Duration::from_millis(30)).await;

    // Heartbeat from runner_hb_a automatically sweeps stale runner_hb_b without taking over its run.
    let hb_a = srv
        .http
        .post(format!(
            "{}/api/v1/workspaces/ws_hb_auto/runners/runner_hb_a/heartbeat",
            srv.base_url
        ))
        .bearer_auth(&owner_tok)
        .send()
        .await
        .expect("heartbeat a");
    assert_eq!(hb_a.status(), StatusCode::OK);

    let snap = get_snapshot(&srv, &owner_tok, "ws_hb_auto").await;
    let rb = snap
        .runners
        .iter()
        .find(|r| r.id == RunnerId::from("runner_hb_b"))
        .expect("runner_hb_b");
    assert_eq!(rb.status, RunnerStatus::Unreachable);
    assert!(snap.runs[0].ownership.runner_unreachable);
    assert_eq!(
        snap.runs[0].ownership.assigned_runner_id,
        Some(RunnerId::from("runner_hb_b"))
    );

    // Background sweeper helper also sweeps remaining stale Online runners (runner_hb_a).
    tokio::time::sleep(Duration::from_millis(30)).await;
    let pool = create_pool(&srv.test_pg_url).expect("pool");
    let now_ms = runlark_server::config::ServerConfig::current_timestamp_ms();
    let swept = runners::sweep_all_stale_runners(
        &pool,
        Timestamp(now_ms.saturating_sub(15)),
        Timestamp(now_ms),
    )
    .await
    .expect("sweep all");
    assert!(swept.iter().any(|r| r.id == RunnerId::from("runner_hb_a")));
}

#[tokio::test]
async fn test_concurrent_invite_accept_revoke_no_deadlock() {
    let srv = spawn_test_server().await;
    let tok = bootstrap_user(&srv, "usr_inv_dl", "Owner").await;
    create_ws(&srv, &tok, "ws_inv_dl", "ws-inv-dl").await;

    // Concurrent accept vs revoke on multiple invitations must resolve cleanly without 500 deadlock.
    for i in 0..6_u32 {
        let (inv_id, inv_tok) =
            create_invite(&srv, &tok, "ws_inv_dl", &format!("u{i}@ex.com"), "member").await;
        let accept_fut = srv
            .http
            .post(format!(
                "{}/api/v1/invitations/{inv_tok}/accept",
                srv.base_url
            ))
            .json(&json!({ "user_id": format!("usr_inv_race_{i}"), "display_name": "Race" }))
            .send();
        let revoke_fut = srv
            .http
            .post(format!(
                "{}/api/v1/workspaces/ws_inv_dl/invitations/{inv_id}/revoke",
                srv.base_url
            ))
            .bearer_auth(&tok)
            .send();
        let (acc_res, rev_res) = tokio::join!(accept_fut, revoke_fut);
        let acc_status = acc_res.expect("accept").status();
        let rev_status = rev_res.expect("revoke").status();
        assert!(
            (acc_status == StatusCode::OK && rev_status == StatusCode::CONFLICT)
                || (acc_status == StatusCode::CONFLICT && rev_status == StatusCode::OK),
            "unexpected concurrent invite statuses: accept={acc_status}, revoke={rev_status}"
        );
    }
}

fn pinned_accept_cmd(cmd_id: &str, dec_id: &str, policy: &str, digest: &str) -> TeamCommand {
    TeamCommand::RecordDecision(RecordDecisionCommand {
        command_id: CommandId::from(cmd_id),
        decision_id: DecisionId::from(dec_id),
        task_id: TaskId::from("task_pin_1"),
        run_id: RunId::from("run_pin_1"),
        kind: DecisionKind::Accept,
        actor: "Owner".to_string(),
        plan_version: 1,
        head_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        base_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        policy_digest: PolicyDigest::from(policy),
        evidence_digest: ArtifactDigest::parse(digest).expect("digest"),
        comment: Some("Verify pinned policy".to_string()),
        expected_revision: Revision(3),
        created_at: Timestamp(300),
    })
}

async fn upload_and_attach_pin_evidence(srv: &super::harness::TestServer, tok: &str) -> String {
    let payload = b"pinned-policy-evidence";
    let digest = sha256_hex(payload);
    assert_eq!(
        srv.http
            .put(format!(
                "{}/api/v1/workspaces/ws_pin/artifacts/{digest}?kind=evidence_bundle",
                srv.base_url
            ))
            .bearer_auth(tok)
            .body(payload.to_vec())
            .send()
            .await
            .expect("upload cas")
            .status(),
        StatusCode::CREATED
    );
    let attach = TeamCommand::AttachArtifact(AttachArtifactCommand {
        command_id: CommandId::from("cmd_pin_att"),
        run_id: RunId::from("run_pin_1"),
        digest_sha256: ArtifactDigest::parse(&digest).expect("digest"),
        kind: ArtifactKind::EvidenceBundle,
        media_type: "application/json".to_string(),
        size_bytes: payload.len() as u64,
        label: "evidence_bundle".to_string(),
        created_at: Timestamp(140),
    });
    assert_eq!(
        post_command(srv, tok, "ws_pin", &attach).await.status(),
        StatusCode::CREATED
    );
    digest
}

async fn setup_pinned_acceptance_run(srv: &super::harness::TestServer, tok: &str) -> String {
    register_runner_ok(srv, tok, "ws_pin", "runner_pin").await;

    assert_eq!(
        post_command(
            srv,
            tok,
            "ws_pin",
            &sample_task_cmd("cmd_pin_t1", "task_pin_1", "proj_pin", "Pinned policy")
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        post_command(
            srv,
            tok,
            "ws_pin",
            &sample_run_cmd(
                "cmd_pin_r1",
                "run_pin_1",
                "proj_pin",
                "task_pin_1",
                Some("runner_pin")
            )
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    let mv = TeamCommand::MoveTaskStage(MoveTaskStageCommand {
        command_id: CommandId::from("cmd_pin_mv"),
        task_id: TaskId::from("task_pin_1"),
        target_stage: TaskStage::Acceptance,
        actor: ActorRole::Human("Owner".to_string()),
        expected_revision: Revision(2),
        created_at: Timestamp(120),
    });
    assert_eq!(
        post_command(srv, tok, "ws_pin", &mv).await.status(),
        StatusCode::CREATED
    );

    for stage in [WorkflowStage::Verification, WorkflowStage::HumanAcceptance] {
        let rep = RunnerRunReport {
            runner_id: RunnerId::from("runner_pin"),
            lease_epoch: 1,
            status: None,
            workflow_stage: Some(stage),
            head_sha: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            base_sha: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
            events: vec![],
        };
        let res = srv
            .http
            .post(format!(
                "{}/api/v1/workspaces/ws_pin/runs/run_pin_1/runner-state",
                srv.base_url
            ))
            .bearer_auth(tok)
            .json(&rep)
            .send()
            .await
            .expect("advance");
        assert_eq!(res.status(), StatusCode::OK);
    }

    upload_and_attach_pin_evidence(srv, tok).await
}

#[tokio::test]
async fn test_decision_binds_to_run_pinned_policy_digest() {
    let srv = spawn_test_server().await;
    let tok = bootstrap_user(&srv, "usr_pin", "Owner").await;
    create_ws(&srv, &tok, "ws_pin", "ws-pin").await;
    promote_project(&srv, &tok, "ws_pin", "proj_pin").await;
    let digest = setup_pinned_acceptance_run(&srv, &tok).await;

    // Project policy rotates to pol_v2 while run_pin_1 stays pinned to pol_v1.
    let pool = create_pool(&srv.test_pg_url).expect("pool");
    let client = pool.get().await.expect("client");
    client
        .execute(
            "UPDATE team_projects SET policy_digest = 'pol_v2' WHERE project_id = 'proj_pin'",
            &[],
        )
        .await
        .expect("rotate project policy");

    // Decision binding against project's newer pol_v2 MUST fail; pinned pol_v1 MUST succeed.
    assert_eq!(
        post_command(
            &srv,
            &tok,
            "ws_pin",
            &pinned_accept_cmd("cmd_pin_bad", "dec_pin_bad", "pol_v2", &digest)
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        post_command(
            &srv,
            &tok,
            "ws_pin",
            &pinned_accept_cmd("cmd_pin_ok", "dec_pin_ok", "pol_v1", &digest)
        )
        .await
        .status(),
        StatusCode::CREATED
    );
}
