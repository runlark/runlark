use reqwest::Client;
use runlark_cli::{
    AppState, PipelineError, VerificationService, VerifyPipelineOptions, VerifyPipelineOutcome,
    WorkflowPipeline, build_router,
};
use runlark_domain::{
    BudgetBlockReason, CursorSeq, RunId, RunStatus, RunnerId, ServerPromotionAck, Timestamp,
    UserId, WorkflowStage, WorkspaceId,
};
use runlark_local::{
    ArtifactCasStore, EventStore, SyncStore, TeamAuthSession, TeamAuthStore, initialize_schema,
    open_sqlite,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn solo_default_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("workflows")
        .join("solo-default")
}

fn init_git_repo(dir: &Path) {
    let run = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(out.status.success(), "git {args:?} failed");
    };
    run(&["init"]);
    run(&["config", "user.email", "test@runlark.dev"]);
    run(&["config", "user.name", "Runlark Test"]);
    fs::write(dir.join("lib.rs"), "pub fn v() -> u32 { 1 }\n").expect("write");
    run(&["add", "lib.rs"]);
    run(&["commit", "-m", "init"]);
}

async fn setup_repair_harness(
    temp: &TempDir,
) -> (
    AppState,
    Client,
    String,
    PathBuf,
    String,
    tokio::task::JoinHandle<()>,
) {
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("mkdir");
    init_git_repo(&repo_dir);

    let db_path = temp.path().join("runlark.db");
    let cas_dir = temp.path().join("wasm-cas");
    let conn = open_sqlite(&db_path).expect("open db");
    initialize_schema(&conn).expect("schema");
    drop(conn);

    let verification = VerificationService::with_cas_dir(&cas_dir).expect("init verification");
    let pin = verification
        .install_package_dir(&solo_default_dir())
        .expect("install solo-default");

    let state = AppState::with_verification(db_path.clone(), verification);
    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let client = Client::new();
    let base = format!("http://{addr}");
    let proj_res = client
        .post(format!("{base}/api/v1/projects"))
        .json(&json!({
            "project_id": "proj_rep",
            "name": "Repair Test",
            "repo_root": repo_dir.to_string_lossy(),
            "budget_policy": {
                "max_attempts": 2,
                "max_duration_ms": 600_000,
                "max_spend_micro_usd": 1_000_000,
                "allow_unknown_cost_repair": false
            }
        }))
        .send()
        .await
        .expect("proj");
    assert_eq!(proj_res.status(), 201);

    (state, client, base, db_path, pin.package_digest.0, server)
}

#[tokio::test]
async fn verification_pins_digest_and_stores_cas_artifacts() {
    let temp = TempDir::new().expect("temp dir");
    let (state, client, base, _db_path, pinned_digest, server) = setup_repair_harness(&temp).await;

    client
        .post(format!("{base}/api/v1/tasks"))
        .json(&json!({ "task_id": "task_pass", "project_id": "proj_rep", "title": "QuickFix pass", "kind": "quick_fix" }))
        .send()
        .await
        .expect("t1");
    let start1: Value = client
        .post(format!("{base}/api/v1/tasks/task_pass/start"))
        .json(&json!({ "run_id": "run_pass_1", "session_id": "sess_pass_1" }))
        .send()
        .await
        .expect("start1")
        .json()
        .await
        .expect("json");
    assert_eq!(start1["run"]["workflow_digest"], pinned_digest);
    assert_eq!(start1["run"]["workflow_stage"], "implementation");

    let wt1 = PathBuf::from(start1["run"]["worktree_path"].as_str().expect("wt1"));
    fs::write(wt1.join("lib.rs"), "pub fn v() -> u32 { 2 }\n").expect("edit worktree");

    let pass_outcome = WorkflowPipeline::verify_and_advance(
        &state,
        &RunId::from("run_pass_1"),
        VerifyPipelineOptions {
            check_command: Some("true".to_string()),
            ..VerifyPipelineOptions::default()
        },
        Timestamp(5_000),
    )
    .await
    .expect("verify pass");
    match pass_outcome {
        VerifyPipelineOutcome::PassedForAcceptance {
            workflow_stage,
            run_status,
            diff_digest,
            verification_log_digest,
            wasm_validation_digest,
            evidence_digest,
            ..
        } => {
            assert_eq!(workflow_stage, WorkflowStage::HumanAcceptance);
            assert_eq!(run_status, RunStatus::WaitingForApproval);
            let cas = ArtifactCasStore::from_data_dir(temp.path());
            assert!(cas.get_bytes(&diff_digest).is_ok());
            assert!(cas.get_bytes(&verification_log_digest).is_ok());
            assert!(cas.get_bytes(&wasm_validation_digest).is_ok());
            assert!(cas.get_bytes(&evidence_digest).is_ok());
        }
        other => panic!("expected PassedForAcceptance, got {other:?}"),
    }

    let t1_after: Value = client
        .get(format!("{base}/api/v1/tasks/task_pass"))
        .send()
        .await
        .expect("get t1")
        .json()
        .await
        .expect("json");
    assert_eq!(t1_after["task"]["stage"], "acceptance");
    assert_eq!(t1_after["latest_run"]["status"], "WaitingForApproval");
    server.abort();
}

#[tokio::test]
async fn repair_loop_enforces_max_attempts_budget() {
    let temp = TempDir::new().expect("temp dir");
    let (state, client, base, _db_path, _digest, server) = setup_repair_harness(&temp).await;

    client
        .post(format!("{base}/api/v1/tasks"))
        .json(&json!({ "task_id": "task_fail_budget", "project_id": "proj_rep", "title": "Failing check repair loop", "kind": "quick_fix" }))
        .send()
        .await
        .expect("t2");
    let start2: Value = client
        .post(format!("{base}/api/v1/tasks/task_fail_budget/start"))
        .json(&json!({ "run_id": "run_rep_1", "session_id": "sess_rep_1" }))
        .send()
        .await
        .expect("start2")
        .json()
        .await
        .expect("json");
    let rev_run2 = start2["run"]["revision"].as_u64().expect("rev");

    client
        .post(format!("{base}/api/v1/runs/run_rep_1/attempts"))
        .json(&json!({
            "command_id": "cmd_att_1",
            "attempt_id": "att_1",
            "workflow_stage": "implementation",
            "outcome": { "status": "failed", "error": "initial check failed" },
            "cost_micro_usd": 10_000,
            "expected_revision": rev_run2
        }))
        .send()
        .await
        .expect("att1");

    let rep1 = WorkflowPipeline::verify_and_advance(
        &state,
        &RunId::from("run_rep_1"),
        VerifyPipelineOptions {
            check_command: Some("false".to_string()),
            ..VerifyPipelineOptions::default()
        },
        Timestamp(6_000),
    )
    .await
    .expect("rep1");
    assert!(matches!(
        rep1,
        VerifyPipelineOutcome::RepairScheduled {
            repair_attempt: 1,
            ..
        }
    ));

    let run2_now: Value = client
        .get(format!("{base}/api/v1/runs/run_rep_1"))
        .send()
        .await
        .expect("run2")
        .json()
        .await
        .expect("json");
    client
        .post(format!("{base}/api/v1/runs/run_rep_1/attempts"))
        .json(&json!({
            "command_id": "cmd_att_2",
            "attempt_id": "att_2",
            "workflow_stage": "implementation",
            "outcome": { "status": "failed", "error": "still failing" },
            "cost_micro_usd": 10_000,
            "expected_revision": run2_now["revision"]
        }))
        .send()
        .await
        .expect("att2");

    let rep2 = WorkflowPipeline::verify_and_advance(
        &state,
        &RunId::from("run_rep_1"),
        VerifyPipelineOptions {
            check_command: Some("false".to_string()),
            ..VerifyPipelineOptions::default()
        },
        Timestamp(7_000),
    )
    .await
    .expect("rep2");
    assert!(matches!(
        rep2,
        VerifyPipelineOutcome::BudgetHalted {
            block_reason: BudgetBlockReason::MaxAttemptsReached { used: 2, max: 2 },
            ..
        }
    ));
    server.abort();
}

#[tokio::test]
async fn unknown_cost_attempt_blocks_repair_and_emits_explicit_events() {
    let temp = TempDir::new().expect("temp dir");
    let (state, client, base, db_path, _digest, server) = setup_repair_harness(&temp).await;

    client
        .post(format!("{base}/api/v1/tasks"))
        .json(&json!({ "task_id": "task_unk", "project_id": "proj_rep", "title": "Unknown cost guard", "kind": "quick_fix" }))
        .send()
        .await
        .expect("t3");
    let start3: Value = client
        .post(format!("{base}/api/v1/tasks/task_unk/start"))
        .json(&json!({ "run_id": "run_unk_1", "session_id": "sess_unk_1" }))
        .send()
        .await
        .expect("start3")
        .json()
        .await
        .expect("json");

    let att_unk: Value = client
        .post(format!("{base}/api/v1/runs/run_unk_1/attempts"))
        .json(&json!({
            "command_id": "cmd_att_unk",
            "attempt_id": "att_unk",
            "workflow_stage": "implementation",
            "outcome": { "status": "succeeded" },
            "cost_micro_usd": null,
            "unknown_cost_reason": "provider omitted turn pricing",
            "expected_revision": start3["run"]["revision"]
        }))
        .send()
        .await
        .expect("att_unk")
        .json()
        .await
        .expect("json");
    assert_eq!(att_unk["cost_summary"]["unknown_attempts"], 1);
    assert_eq!(att_unk["cost_summary"]["known_micro_usd"], 0);

    let halted_unk = WorkflowPipeline::verify_and_advance(
        &state,
        &RunId::from("run_unk_1"),
        VerifyPipelineOptions {
            check_command: Some("false".to_string()),
            ..VerifyPipelineOptions::default()
        },
        Timestamp(8_000),
    )
    .await
    .expect("halt unk");
    assert!(matches!(
        halted_unk,
        VerifyPipelineOutcome::BudgetHalted {
            block_reason: BudgetBlockReason::UnknownCostBlocked {
                unknown_attempts: 1
            },
            ..
        }
    ));

    let conn = open_sqlite(&db_path).expect("open");
    let events = EventStore::get_events(&conn, &RunId::from("run_unk_1"), None).expect("events");
    assert!(events.iter().any(|e| e.event_type == "UnknownCostRecorded"));
    assert!(events.iter().any(|e| e.event_type == "BudgetExceeded"));
    assert!(events.iter().any(|e| e.event_type == "UnknownCostBlocked"));

    server.abort();
}

fn seed_team_leased_run(db_path: &Path, reject_url: &str, wt_path: &Path) {
    let mut conn = open_sqlite(db_path).expect("open");
    let wid = WorkspaceId::from("ws_pub");
    SyncStore::promote_project_to_team_atomic(
        &mut conn,
        &runlark_domain::ProjectId::from("proj_rep"),
        reject_url,
        &ServerPromotionAck {
            workspace_id: wid.clone(),
            project_id: runlark_domain::ProjectId::from("proj_rep"),
            ownership_epoch: 1,
            initial_cursor: CursorSeq(0),
            acknowledged_at: Timestamp(1_000),
        },
        Timestamp(1_000),
    )
    .expect("promote");
    TeamAuthStore::save_session(
        &conn,
        &TeamAuthSession {
            server_url: reject_url.to_string(),
            user_id: UserId::from("usr_pub"),
            display_name: "Pub".to_string(),
            bearer_token: "tok".to_string(),
            default_workspace_id: Some(wid),
            runner_id: Some(RunnerId::from("rnr_pub")),
            runner_name: Some("Pub Runner".to_string()),
            updated_at: Timestamp(1_000),
        },
    )
    .expect("save auth");
    runlark_local::SessionStore::create_session(
        &conn,
        &runlark_domain::Session {
            id: runlark_domain::SessionId::from("sess_pub_1"),
            task_id: runlark_domain::TaskId::from("task_pub"),
            cwd: wt_path.to_string_lossy().to_string(),
            status: runlark_domain::SessionStatus::Active,
            current_revision: runlark_domain::Revision(1),
            active_run_id: Some(RunId::from("run_pub_1")),
            created_at: Timestamp(1_000),
            updated_at: Timestamp(1_000),
        },
    )
    .expect("session");
    runlark_local::SessionStore::save_run(
        &conn,
        &runlark_domain::Run {
            id: RunId::from("run_pub_1"),
            session_id: runlark_domain::SessionId::from("sess_pub_1"),
            status: RunStatus::Running,
            worktree_path: wt_path.to_string_lossy().to_string(),
            revision: runlark_domain::Revision(1),
            created_at: Timestamp(1_000),
            updated_at: Timestamp(1_000),
            task_id: Some(runlark_domain::TaskId::from("task_pub")),
            project_id: Some(runlark_domain::ProjectId::from("proj_rep")),
            workflow_stage: WorkflowStage::Clarification,
            task_kind: runlark_domain::TaskKind::Standard,
            workflow_digest: None,
            base_sha: None,
            head_sha: None,
            plan_version: 1,
            policy_digest: Some(runlark_domain::PolicyDigest::from("pol_pub")),
            budget_policy: runlark_domain::BudgetPolicy::default(),
            cost_summary: runlark_domain::CostSummary::default(),
            repair_attempts: 0,
            pr_url: None,
        },
    )
    .expect("run");
    conn.execute(
        "INSERT INTO run_ownership (run_id, assigned_runner_id, lease_epoch, runner_unreachable, updated_at) VALUES (?1, ?2, 1, 0, 1000)",
        ["run_pub_1", "rnr_pub"],
    )
    .expect("upsert ownership");
}

#[tokio::test]
async fn committed_stage_and_verify_publish_events_and_notify_even_if_team_report_fails() {
    let temp = TempDir::new().expect("temp dir");
    let (state, client, base, db_path, _digest, server) = setup_repair_harness(&temp).await;

    let reject_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let reject_addr = reject_listener.local_addr().expect("addr");
    let reject_app = axum::Router::new().fallback(|| async {
        (
            axum::http::StatusCode::CONFLICT,
            axum::Json(json!({ "error": "runner_fencing_mismatch", "message": "stale" })),
        )
    });
    let reject_server = tokio::spawn(async move {
        let _ = axum::serve(reject_listener, reject_app).await;
    });
    let reject_url = format!("http://{reject_addr}");

    let wt_path = temp.path().join("wt_pub");
    fs::create_dir_all(&wt_path).expect("mkdir wt");
    init_git_repo(&wt_path);
    fs::write(wt_path.join("lib.rs"), "pub fn v() -> u32 { 3 }\n").expect("edit");

    client
        .post(format!("{base}/api/v1/tasks"))
        .json(&json!({ "task_id": "task_pub", "project_id": "proj_rep", "title": "Publish on error", "kind": "standard" }))
        .send()
        .await
        .expect("t");
    seed_team_leased_run(&db_path, &reject_url, &wt_path);

    let mut rx = state.subscribe_events();
    let adv_err = WorkflowPipeline::advance_stage_with_task(
        &state,
        &RunId::from("run_pub_1"),
        WorkflowStage::Plan,
        Timestamp(2_000),
    )
    .await
    .expect_err("team report conflict must return Err");
    assert!(matches!(adv_err, PipelineError::TeamFencing(_)));
    let adv_ev = rx
        .try_recv()
        .expect("advance_stage event must be published");
    assert_eq!(adv_ev.session_id, "sess_pub_1");

    // Advance directly to Implementation so verify_and_advance transitions to HumanAcceptance.
    let _ = WorkflowPipeline::advance_stage_with_task(
        &state,
        &RunId::from("run_pub_1"),
        WorkflowStage::Implementation,
        Timestamp(3_000),
    )
    .await;
    while rx.try_recv().is_ok() {}

    let sched_fut = state.scheduler_notify.notified();
    let ver_err = WorkflowPipeline::verify_and_advance(
        &state,
        &RunId::from("run_pub_1"),
        VerifyPipelineOptions {
            check_command: Some("true".to_string()),
            ..VerifyPipelineOptions::default()
        },
        Timestamp(4_000),
    )
    .await
    .expect_err("team report conflict must return Err");
    assert!(matches!(ver_err, PipelineError::TeamFencing(_)));
    assert!(
        rx.try_recv().is_ok(),
        "verify_and_advance events must be published"
    );
    tokio::time::timeout(std::time::Duration::from_millis(50), sched_fut)
        .await
        .expect("scheduler must be notified despite team_report failure");

    reject_server.abort();
    server.abort();
}
