use super::harness::{
    TestServer, accept_invite, bootstrap_user, create_invite, create_ws, get_snapshot,
    post_command, promote_project, sample_run_cmd, sample_task_cmd, spawn_test_server,
};
use reqwest::StatusCode;
use runlark_domain::{
    ActorRole, AddCommentCommand, ArtifactDigest, ArtifactKind, AttachArtifactCommand, CommandId,
    CommentId, CreateTaskCommand, CreateTeamRunCommand, DecisionId, DecisionKind, LeaseEpoch,
    MoveTaskStageCommand, PolicyDigest, ProjectId, RecordDecisionCommand, Revision, RunId,
    RunStatus, RunnerId, RunnerRunReport, TaskComment, TaskId, TaskKind, TaskStage, TeamCommand,
    Timestamp, WorkflowStage, WorkspaceEventEnvelope,
};
use runlark_server::cas::sha256_hex;
use serde_json::json;

fn create_task_cmd(cmd_id: &str, task_id: &str, project_id: &str, title: &str) -> TeamCommand {
    TeamCommand::CreateTask(CreateTaskCommand {
        command_id: CommandId::from(cmd_id),
        task_id: TaskId::from(task_id),
        project_id: ProjectId::from(project_id),
        title: title.to_string(),
        description: "Verify CAS & decisions".to_string(),
        kind: TaskKind::QuickFix,
        base_ref: Some("main".to_string()),
        budget_override: None,
        created_at: Timestamp(100),
    })
}

fn accept_decision_cmd(
    cmd_id: &str,
    dec_id: &str,
    task_id: &str,
    run_id: &str,
    digest: &str,
    expected_revision: Revision,
) -> TeamCommand {
    TeamCommand::RecordDecision(RecordDecisionCommand {
        command_id: CommandId::from(cmd_id),
        decision_id: DecisionId::from(dec_id),
        task_id: TaskId::from(task_id),
        run_id: RunId::from(run_id),
        kind: DecisionKind::Accept,
        actor: "Syncer".to_string(),
        plan_version: 1,
        head_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        base_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        policy_digest: PolicyDigest::from("pol_v1"),
        evidence_digest: ArtifactDigest::parse(digest).expect("digest"),
        comment: Some("LGTM".to_string()),
        expected_revision,
        created_at: Timestamp(150),
    })
}

async fn setup_task_and_run_for_acceptance(srv: &TestServer, tok: &str, wid: &str, pid: &str) {
    assert_eq!(
        post_command(
            srv,
            tok,
            wid,
            &create_task_cmd("cmd_sync_t1", "task_sync_1", pid, "Sync task")
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    let r = srv
        .http
        .post(format!(
            "{}/api/v1/workspaces/{wid}/runners/register",
            srv.base_url
        ))
        .bearer_auth(tok)
        .json(&json!({ "runner_id": "runner_sync", "name": "Sync Runner" }))
        .send()
        .await
        .expect("register runner");
    assert_eq!(r.status(), StatusCode::OK);

    let run_cmd = TeamCommand::CreateTeamRun(CreateTeamRunCommand {
        command_id: CommandId::from("cmd_sync_r1"),
        run_id: RunId::from("run_sync_1"),
        project_id: ProjectId::from(pid),
        task_id: TaskId::from("task_sync_1"),
        runner_id: Some(RunnerId::from("runner_sync")),
        expected_revision: Revision(1),
        created_at: Timestamp(120),
    });
    assert_eq!(
        post_command(srv, tok, wid, &run_cmd).await.status(),
        StatusCode::CREATED
    );

    let mv_accept = TeamCommand::MoveTaskStage(MoveTaskStageCommand {
        command_id: CommandId::from("cmd_mv_accept"),
        task_id: TaskId::from("task_sync_1"),
        target_stage: TaskStage::Acceptance,
        actor: ActorRole::Human("Syncer".to_string()),
        expected_revision: Revision(2),
        created_at: Timestamp(130),
    });
    assert_eq!(
        post_command(srv, tok, wid, &mv_accept).await.status(),
        StatusCode::CREATED
    );
}

async fn advance_run_to_human_acceptance(srv: &TestServer, tok: &str, wid: &str, run_id: &str) {
    for (stage, head, base) in [
        (WorkflowStage::Verification, None, None),
        (
            WorkflowStage::HumanAcceptance,
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
        ),
    ] {
        let res = srv
            .http
            .post(format!(
                "{}/api/v1/workspaces/{wid}/runs/{run_id}/runner-state",
                srv.base_url
            ))
            .bearer_auth(tok)
            .json(&RunnerRunReport {
                runner_id: RunnerId::from("runner_sync"),
                lease_epoch: LeaseEpoch(1),
                status: None,
                workflow_stage: Some(stage),
                head_sha: head,
                base_sha: base,
                events: vec![],
            })
            .send()
            .await
            .expect("advance stage");
        assert_eq!(res.status(), StatusCode::OK);
    }
}

async fn upload_and_attach_evidence(
    srv: &TestServer,
    tok: &str,
    wid: &str,
    run_id: &str,
    payload: &[u8],
) -> String {
    let digest = sha256_hex(payload);
    assert_eq!(
        srv.http
            .put(format!(
                "{}/api/v1/workspaces/{wid}/artifacts/{digest}?kind=evidence_bundle",
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
        command_id: CommandId::from("cmd_att_1"),
        run_id: RunId::from(run_id),
        digest_sha256: ArtifactDigest::parse(&digest).expect("digest"),
        kind: ArtifactKind::EvidenceBundle,
        media_type: "application/json".to_string(),
        size_bytes: payload.len() as u64,
        label: "evidence_bundle".to_string(),
        created_at: Timestamp(140),
    });
    let att_res = post_command(srv, tok, wid, &attach).await;
    assert_eq!(att_res.status(), StatusCode::CREATED);
    let att_json: serde_json::Value = att_res.json().await.expect("attach json");
    assert_eq!(att_json["revision"].as_u64(), Some(2));
    digest
}

#[tokio::test]
async fn test_command_idempotency_conflict_and_project_check() {
    let srv = spawn_test_server().await;
    let tok = bootstrap_user(&srv, "usr_occ", "OCC User").await;
    create_ws(&srv, &tok, "ws_occ", "ws-occ").await;
    promote_project(&srv, &tok, "ws_occ", "proj_occ_1").await;
    promote_project(&srv, &tok, "ws_occ", "proj_occ_2").await;

    let cmd1 = create_task_cmd("cmd_occ_t1", "task_occ_1", "proj_occ_1", "Sync task");
    assert_eq!(
        post_command(&srv, &tok, "ws_occ", &cmd1).await.status(),
        StatusCode::CREATED
    );
    assert_eq!(
        post_command(&srv, &tok, "ws_occ", &cmd1).await.status(),
        StatusCode::OK
    );

    let conflict_cmd = create_task_cmd("cmd_occ_t1", "task_occ_1", "proj_occ_1", "Mutated title!");
    assert_eq!(
        post_command(&srv, &tok, "ws_occ", &conflict_cmd)
            .await
            .status(),
        StatusCode::CONFLICT
    );

    // Cross-project CreateTeamRun must fail with 400 Bad Request.
    let cross_proj_run = sample_run_cmd(
        "cmd_cross_proj",
        "run_cross_proj",
        "proj_occ_2",
        "task_occ_1",
        None,
    );
    assert_eq!(
        post_command(&srv, &tok, "ws_occ", &cross_proj_run)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn test_move_task_stage_actor_check_and_queued_cancel_sync() {
    let srv = spawn_test_server().await;
    let tok = bootstrap_user(&srv, "usr_mv", "Mover").await;
    create_ws(&srv, &tok, "ws_mv", "ws-mv").await;
    promote_project(&srv, &tok, "ws_mv", "proj_mv").await;

    assert_eq!(
        post_command(
            &srv,
            &tok,
            "ws_mv",
            &create_task_cmd("cmd_mv_t1", "task_mv_1", "proj_mv", "Actor check")
        )
        .await
        .status(),
        StatusCode::CREATED
    );

    for (cid, stage, rev) in [
        ("cmd_mv_ready", TaskStage::Ready, 1),
        ("cmd_mv_ip", TaskStage::InProgress, 2),
    ] {
        let mv = TeamCommand::MoveTaskStage(MoveTaskStageCommand {
            command_id: CommandId::from(cid),
            task_id: TaskId::from("task_mv_1"),
            target_stage: stage,
            actor: ActorRole::Human("Mover".to_string()),
            expected_revision: Revision(rev),
            created_at: Timestamp(120),
        });
        assert_eq!(
            post_command(&srv, &tok, "ws_mv", &mv).await.status(),
            StatusCode::CREATED
        );
    }

    let agent_done = TeamCommand::MoveTaskStage(MoveTaskStageCommand {
        command_id: CommandId::from("cmd_agent_done"),
        task_id: TaskId::from("task_mv_1"),
        target_stage: TaskStage::Done,
        actor: ActorRole::Agent("bot".to_string()),
        expected_revision: Revision(3),
        created_at: Timestamp(122),
    });
    assert_eq!(
        post_command(&srv, &tok, "ws_mv", &agent_done)
            .await
            .status(),
        StatusCode::CONFLICT
    );

    // Unassigned queued run cancelled via CancelRun settles to Stopped and clears task.active_run_id.
    assert_eq!(
        post_command(
            &srv,
            &tok,
            "ws_mv",
            &sample_task_cmd("cmd_mv_t2", "task_mv_2", "proj_mv", "Queued cancel")
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        post_command(
            &srv,
            &tok,
            "ws_mv",
            &sample_run_cmd("cmd_mv_r2", "run_mv_2", "proj_mv", "task_mv_2", None)
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    let cancel_q = TeamCommand::CancelRun(runlark_domain::CancelTeamRunCommand {
        command_id: CommandId::from("cmd_mv_cancel_q"),
        run_id: RunId::from("run_mv_2"),
        reason: "cancel queued".to_string(),
        expected_revision: Revision(1),
        created_at: Timestamp(130),
    });
    assert_eq!(
        post_command(&srv, &tok, "ws_mv", &cancel_q).await.status(),
        StatusCode::CREATED
    );
    let snap = get_snapshot(&srv, &tok, "ws_mv").await;
    let t2 = snap
        .tasks
        .iter()
        .find(|t| t.id == TaskId::from("task_mv_2"))
        .expect("task_mv_2");
    assert_eq!(t2.active_run_id, None);
    assert_eq!(t2.revision, Revision(3));
    assert_eq!(snap.runs[0].run.status, RunStatus::Stopped);
}

#[tokio::test]
async fn test_cas_auth_before_disk_write_and_workspace_isolation() {
    let srv = spawn_test_server().await;
    let tok_a = bootstrap_user(&srv, "usr_cas_a", "Alice").await;
    let tok_b = bootstrap_user(&srv, "usr_cas_b", "Bob").await;
    create_ws(&srv, &tok_a, "ws_cas_a", "ws-cas-a").await;
    create_ws(&srv, &tok_b, "ws_cas_b", "ws-cas-b").await;

    let (_, viewer_inv) = create_invite(&srv, &tok_a, "ws_cas_a", "v@example.com", "viewer").await;
    let viewer_tok = accept_invite(&srv, &viewer_inv, "usr_cas_viewer", "Viewer").await;

    let viewer_secret = b"viewer-must-not-write-disk";
    let viewer_digest = sha256_hex(viewer_secret);
    assert_eq!(
        srv.http
            .put(format!(
                "{}/api/v1/workspaces/ws_cas_a/artifacts/{viewer_digest}?kind=evidence_bundle",
                srv.base_url
            ))
            .bearer_auth(&viewer_tok)
            .body(viewer_secret.to_vec())
            .send()
            .await
            .expect("viewer upload")
            .status(),
        StatusCode::FORBIDDEN
    );
    assert!(!srv.cas_root.join("sha256").join(&viewer_digest).exists());

    let payload = b"verified-evidence-bundle-v1";
    let digest = sha256_hex(payload);
    let bad_digest = "0".repeat(64);
    assert_eq!(
        srv.http
            .put(format!(
                "{}/api/v1/workspaces/ws_cas_a/artifacts/{bad_digest}?kind=evidence_bundle",
                srv.base_url
            ))
            .bearer_auth(&tok_a)
            .body(payload.to_vec())
            .send()
            .await
            .expect("bad digest")
            .status(),
        StatusCode::BAD_REQUEST
    );

    assert_eq!(
        srv.http
            .put(format!(
                "{}/api/v1/workspaces/ws_cas_a/artifacts/{digest}?kind=evidence_bundle",
                srv.base_url
            ))
            .bearer_auth(&tok_a)
            .body(payload.to_vec())
            .send()
            .await
            .expect("upload cas")
            .status(),
        StatusCode::CREATED
    );
    let downloaded = srv
        .http
        .get(format!(
            "{}/api/v1/workspaces/ws_cas_a/artifacts/{digest}",
            srv.base_url
        ))
        .bearer_auth(&tok_a)
        .send()
        .await
        .expect("download cas")
        .bytes()
        .await
        .expect("bytes");
    assert_eq!(downloaded.as_ref(), payload);

    assert_eq!(
        srv.http
            .get(format!(
                "{}/api/v1/workspaces/ws_cas_b/artifacts/{digest}",
                srv.base_url
            ))
            .bearer_auth(&tok_b)
            .send()
            .await
            .expect("cross-ws download")
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn test_comments_decisions_unbacked_attach_rejection_and_snapshot_cursor() {
    let srv = spawn_test_server().await;
    let tok = bootstrap_user(&srv, "usr_sync", "Syncer").await;
    create_ws(&srv, &tok, "ws_sync", "ws-sync").await;
    promote_project(&srv, &tok, "ws_sync", "proj_sync").await;
    setup_task_and_run_for_acceptance(&srv, &tok, "ws_sync", "proj_sync").await;

    let cmt = TeamCommand::AddComment(AddCommentCommand {
        command_id: CommandId::from("cmd_cmt_1"),
        comment_id: CommentId::from("cmt_1"),
        task_id: TaskId::from("task_sync_1"),
        run_id: None,
        body: "First append-only comment".to_string(),
        created_at: Timestamp(110),
    });
    assert_eq!(
        post_command(&srv, &tok, "ws_sync", &cmt).await.status(),
        StatusCode::CREATED
    );
    let comments: Vec<TaskComment> = srv
        .http
        .get(format!(
            "{}/api/v1/workspaces/ws_sync/tasks/task_sync_1/comments",
            srv.base_url
        ))
        .bearer_auth(&tok)
        .send()
        .await
        .expect("list comments")
        .json()
        .await
        .expect("comments json");
    assert_eq!(comments.len(), 1);

    // Attaching an unuploaded digest MUST fail with 409 Conflict.
    let payload = b"verified-evidence-bundle-v2";
    let unuploaded_digest = sha256_hex(payload);
    let unbacked_attach = TeamCommand::AttachArtifact(AttachArtifactCommand {
        command_id: CommandId::from("cmd_att_missing"),
        run_id: RunId::from("run_sync_1"),
        digest_sha256: ArtifactDigest::parse(&unuploaded_digest).expect("digest"),
        kind: ArtifactKind::EvidenceBundle,
        media_type: "application/json".to_string(),
        size_bytes: payload.len() as u64,
        label: "evidence_bundle".to_string(),
        created_at: Timestamp(132),
    });
    assert_eq!(
        post_command(&srv, &tok, "ws_sync", &unbacked_attach)
            .await
            .status(),
        StatusCode::CONFLICT
    );

    // Upload & attach artifact, but Accept MUST still fail while run lacks SHAs/HumanAcceptance.
    let digest = upload_and_attach_evidence(&srv, &tok, "ws_sync", "run_sync_1", payload).await;
    let premature_accept = accept_decision_cmd(
        "cmd_dec_premature",
        "dec_premature",
        "task_sync_1",
        "run_sync_1",
        &digest,
        Revision(3),
    );
    assert_eq!(
        post_command(&srv, &tok, "ws_sync", &premature_accept)
            .await
            .status(),
        StatusCode::CONFLICT
    );

    // Advance run through Verification -> HumanAcceptance with recorded head/base SHAs, then Accept.
    advance_run_to_human_acceptance(&srv, &tok, "ws_sync", "run_sync_1").await;
    let accept = accept_decision_cmd(
        "cmd_dec_accept",
        "dec_1",
        "task_sync_1",
        "run_sync_1",
        &digest,
        Revision(3),
    );
    assert_eq!(
        post_command(&srv, &tok, "ws_sync", &accept).await.status(),
        StatusCode::CREATED
    );

    let snap = get_snapshot(&srv, &tok, "ws_sync").await;
    assert_eq!(snap.tasks[0].stage, TaskStage::Done);
    assert_eq!(snap.decisions.len(), 1);
    assert_eq!(snap.comments.len(), 1);

    let delta: Vec<WorkspaceEventEnvelope> = srv
        .http
        .get(format!(
            "{}/api/v1/workspaces/ws_sync/sync/events?after_cursor={}",
            srv.base_url, snap.cursor_seq.0
        ))
        .bearer_auth(&tok)
        .send()
        .await
        .expect("events after snapshot")
        .json()
        .await
        .expect("events json");
    assert!(delta.is_empty());
}

#[tokio::test]
async fn test_repeatable_read_snapshot_under_concurrent_writes() {
    let srv = spawn_test_server().await;
    let tok = bootstrap_user(&srv, "usr_conc", "Concurrent").await;
    create_ws(&srv, &tok, "ws_conc", "ws-conc").await;
    promote_project(&srv, &tok, "ws_conc", "proj_conc").await;
    assert_eq!(
        post_command(
            &srv,
            &tok,
            "ws_conc",
            &create_task_cmd("cmd_conc_t1", "task_conc_1", "proj_conc", "Concurrent task")
        )
        .await
        .status(),
        StatusCode::CREATED
    );

    let mut handles = Vec::new();
    for i in 0..8_u32 {
        let http = srv.http.clone();
        let url = format!("{}/api/v1/workspaces/ws_conc/commands", srv.base_url);
        let token = tok.clone();
        handles.push(tokio::spawn(async move {
            let cmd = TeamCommand::AddComment(AddCommentCommand {
                command_id: CommandId::from(format!("cmd_conc_c_{i}")),
                comment_id: CommentId::from(format!("cmt_conc_{i}")),
                task_id: TaskId::from("task_conc_1"),
                run_id: None,
                body: format!("comment {i}"),
                created_at: Timestamp(u64::from(i) + 100),
            });
            let res = http
                .post(url)
                .bearer_auth(token)
                .json(&cmd)
                .send()
                .await
                .expect("concurrent comment");
            assert_eq!(res.status(), StatusCode::CREATED);
        }));
    }

    let snap = get_snapshot(&srv, &tok, "ws_conc").await;
    for handle in handles {
        handle.await.expect("join concurrent writer");
    }

    let delta: Vec<WorkspaceEventEnvelope> = srv
        .http
        .get(format!(
            "{}/api/v1/workspaces/ws_conc/sync/events?after_cursor={}",
            srv.base_url, snap.cursor_seq.0
        ))
        .bearer_auth(&tok)
        .send()
        .await
        .expect("events after snapshot")
        .json()
        .await
        .expect("delta json");
    assert_eq!(snap.comments.len() + delta.len(), 8);
}
