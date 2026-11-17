use reqwest::Client;
use runlark_cli::{
    AppState, VerificationService, VerifyPipelineOptions, VerifyPipelineOutcome, WorkflowPipeline,
    build_router,
};
use runlark_domain::{BudgetBlockReason, RunId, RunStatus, Timestamp, WorkflowStage};
use runlark_local::{ArtifactCasStore, EventStore, initialize_schema, open_sqlite};
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
