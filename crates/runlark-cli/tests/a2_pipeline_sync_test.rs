mod a2_team_mock;

use a2_team_mock::{
    MockTeamServerState, check_auth, mock_commands, mock_events, mock_snapshot, mock_whoami,
    spawn_daemon,
};
use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path as AxumPath, State as AxumState};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post, put};
use reqwest::Client;
use runlark_cli::{
    AppState, PipelineError, RunOwnershipResponse, RunnerAgentWorker, TeamSyncWorker,
    VerificationService, VerifyPipelineOptions, WorkflowPipeline,
};
use runlark_domain::{
    ArtifactDigest, ArtifactKind, ArtifactRef, BudgetPolicy, CostSummary, CursorSeq, LeaseEpoch,
    PolicyDigest, Project, ProjectId, Revision, Run, RunId, RunOwnership, RunStatus, RunnerId,
    RunnerRegistration, RunnerRunReport, RunnerStatus, ServerPromotionAck, SessionId, Task, TaskId,
    TaskKind, TaskStage, TeamCommand, TeamProjectSpec, TeamRunEntry, Timestamp, UserId,
    WorkflowStage, WorkspaceId,
};
use runlark_local::{
    ProjectStore, RunnerBufferStore, SessionStore, SyncOutboxStore, SyncStore, TeamAuthSession,
    TeamAuthStore, initialize_schema, open_sqlite,
};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tempfile::TempDir;
use tokio::sync::Mutex;

type ReportLog = Arc<Mutex<Vec<(String, RunnerRunReport)>>>;

#[derive(Clone)]
struct PipelineMockState {
    base: MockTeamServerState,
    reports: ReportLog,
    uploaded_digests: Arc<Mutex<Vec<String>>>,
    runner_online: Arc<AtomicBool>,
    heartbeat_reonline: Arc<AtomicBool>,
}

async fn mock_heartbeat(
    AxumState(st): AxumState<PipelineMockState>,
    headers: HeaderMap,
    AxumPath((wid, rid)): AxumPath<(String, String)>,
) -> Result<Json<RunnerRegistration>, (StatusCode, Json<Value>)> {
    check_auth(&headers, &st.base)?;
    if st.heartbeat_reonline.load(Ordering::SeqCst) {
        st.runner_online.store(true, Ordering::SeqCst);
    }
    let status = if st.runner_online.load(Ordering::SeqCst) {
        RunnerStatus::Online
    } else {
        RunnerStatus::Unreachable
    };
    Ok(Json(RunnerRegistration {
        id: RunnerId::from(rid),
        workspace_id: WorkspaceId::from(wid),
        owner_user_id: UserId::from("usr_owner"),
        name: "Mine".to_string(),
        status,
        last_heartbeat_at: Timestamp(2_000),
        created_at: Timestamp(1_000),
    }))
}

async fn mock_runner_state(
    AxumState(st): AxumState<PipelineMockState>,
    headers: HeaderMap,
    AxumPath((_wid, rid)): AxumPath<(String, String)>,
    Json(report): Json<RunnerRunReport>,
) -> Result<Json<RunOwnershipResponse>, (StatusCode, Json<Value>)> {
    check_auth(&headers, &st.base)?;
    if !st.runner_online.load(Ordering::SeqCst) {
        return Err((
            StatusCode::CONFLICT,
            Json(json!({
                "error": "runner_not_online",
                "message": "runner is unreachable; heartbeat first"
            })),
        ));
    }
    st.reports.lock().await.push((rid.clone(), report.clone()));
    let mut data = st.base.inner.lock().await;
    let entry = data
        .snapshot
        .runs
        .iter_mut()
        .find(|e| e.run.id.0 == rid)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "not_found", "message": "run not found" })),
            )
        })?;
    if let Some(st_val) = report.status {
        entry.run.status = st_val;
    }
    if let Some(stage) = report.workflow_stage {
        entry.run.workflow_stage = stage;
        if stage == WorkflowStage::HumanAcceptance {
            entry.run.status = RunStatus::WaitingForApproval;
        }
    }
    if report.head_sha.is_some() {
        entry.run.head_sha = report.head_sha;
    }
    if report.base_sha.is_some() {
        entry.run.base_sha = report.base_sha;
    }
    entry.run.revision = Revision(entry.run.revision.0 + 1);
    Ok(Json(RunOwnershipResponse {
        run: entry.run.clone(),
        ownership: entry.ownership.clone(),
    }))
}

async fn mock_put_pipe_artifact(
    AxumState(st): AxumState<PipelineMockState>,
    h: HeaderMap,
    AxumPath((_w, d)): AxumPath<(String, String)>,
    b: Bytes,
) -> Result<(StatusCode, Json<ArtifactRef>), (StatusCode, Json<Value>)> {
    check_auth(&h, &st.base)?;
    st.uploaded_digests.lock().await.push(d.clone());
    Ok((
        StatusCode::CREATED,
        Json(ArtifactRef {
            digest_sha256: ArtifactDigest::parse(d).unwrap(),
            kind: ArtifactKind::EvidenceBundle,
            size_bytes: b.len() as u64,
            media_type: "application/json".to_string(),
            created_at: Timestamp(1_000),
        }),
    ))
}

async fn mock_pipe_commands(
    AxumState(st): AxumState<PipelineMockState>,
    h: HeaderMap,
    AxumPath(wid): AxumPath<String>,
    Json(cmd): Json<TeamCommand>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    check_auth(&h, &st.base)?;
    if let TeamCommand::AttachArtifact(ref a) = cmd {
        let uploaded = st.uploaded_digests.lock().await;
        if !uploaded.contains(&a.digest_sha256.as_str().to_string()) {
            return Err((
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "artifact_not_uploaded",
                    "message": "blob missing from CAS"
                })),
            ));
        }
    }
    mock_commands(AxumState(st.base.clone()), h, AxumPath(wid), Json(cmd)).await
}

async fn spawn_pipeline_mock() -> (PipelineMockState, String, tokio::task::JoinHandle<()>) {
    let base = MockTeamServerState::new("ws_pipe", "Pipe", "pipe", Some("tok"));
    let state = PipelineMockState {
        base,
        reports: Arc::new(Mutex::new(Vec::new())),
        uploaded_digests: Arc::new(Mutex::new(Vec::new())),
        runner_online: Arc::new(AtomicBool::new(true)),
        heartbeat_reonline: Arc::new(AtomicBool::new(true)),
    };
    let router = Router::new()
        .route(
            "/api/v1/auth/whoami",
            get(|st: AxumState<PipelineMockState>, h| mock_whoami(AxumState(st.base.clone()), h)),
        )
        .route(
            "/api/v1/workspaces/{wid}/sync/snapshot",
            get(|st: AxumState<PipelineMockState>, h| mock_snapshot(AxumState(st.base.clone()), h)),
        )
        .route(
            "/api/v1/workspaces/{wid}/sync/events",
            get(|st: AxumState<PipelineMockState>, h, q| {
                mock_events(AxumState(st.base.clone()), h, q)
            }),
        )
        .route(
            "/api/v1/workspaces/{wid}/commands",
            post(mock_pipe_commands),
        )
        .route(
            "/api/v1/workspaces/{wid}/runners/{rid}/heartbeat",
            post(mock_heartbeat),
        )
        .route(
            "/api/v1/workspaces/{wid}/runs/{rid}/runner-state",
            post(mock_runner_state),
        )
        .route(
            "/api/v1/workspaces/{wid}/artifacts/{digest}",
            put(mock_put_pipe_artifact),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (state, format!("http://{addr}"), handle)
}

fn init_git_repo(repo_dir: &Path) -> String {
    for args in [
        ["init", "-b", "main"].as_slice(),
        ["config", "user.email", "dev@example.com"].as_slice(),
        ["config", "user.name", "Dev"].as_slice(),
    ] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(repo_dir)
                .status()
                .unwrap()
                .success()
        );
    }
    fs::write(repo_dir.join("README.md"), "base\n").unwrap();
    for args in [["add", "."].as_slice(), ["commit", "-m", "init"].as_slice()] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(repo_dir)
                .status()
                .unwrap()
                .success()
        );
    }
    let base_sha = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo_dir)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    fs::write(repo_dir.join("README.md"), "base\nchange\n").unwrap();
    for args in [["add", "."].as_slice(), ["commit", "-m", "feat"].as_slice()] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(repo_dir)
                .status()
                .unwrap()
                .success()
        );
    }
    base_sha
}

async fn seed_state(
    db_path: &Path,
    mock_url: &str,
    mock_state: &MockTeamServerState,
    repo_dir: &Path,
    base_sha: &str,
) {
    let wid = WorkspaceId::from("ws_pipe");
    let pid = ProjectId::from("prj_pipe");
    let mut data = mock_state.inner.lock().await;
    data.snapshot.projects.push(TeamProjectSpec {
        project_id: pid.clone(),
        workspace_id: wid.clone(),
        name: "Pipe".to_string(),
        default_branch: "main".to_string(),
        max_concurrent_runs: 2,
        budget_policy: BudgetPolicy::default(),
        policy_digest: PolicyDigest::from("pol_p"),
        ownership_epoch: 1,
        revision: Revision(1),
        created_at: Timestamp(1_000),
        updated_at: Timestamp(1_000),
    });
    for (tid, wf, rid, owner) in [
        (
            "tsk_verif",
            WorkflowStage::Implementation,
            "run_verif",
            "runner_mine",
        ),
        (
            "tsk_other",
            WorkflowStage::Plan,
            "run_other",
            "runner_other",
        ),
    ] {
        data.snapshot.tasks.push(Task {
            id: TaskId::from(tid),
            project_id: pid.clone(),
            title: tid.to_string(),
            description: "desc".to_string(),
            kind: TaskKind::Standard,
            stage: TaskStage::InProgress,
            workflow_stage: wf,
            plan_text: Some("plan".to_string()),
            plan_version: 1,
            base_ref: Some("main".to_string()),
            head_ref: None,
            pr_url: None,
            active_session_id: Some(SessionId::from(format!("ses_{rid}"))),
            active_run_id: Some(RunId::from(rid)),
            latest_run_id: Some(RunId::from(rid)),
            budget_override: None,
            revision: Revision(1),
            created_at: Timestamp(1_000),
            updated_at: Timestamp(1_000),
        });
        data.snapshot.runs.push(TeamRunEntry {
            run: Run {
                id: RunId::from(rid),
                session_id: SessionId::from(format!("ses_{rid}")),
                status: RunStatus::Running,
                worktree_path: repo_dir.to_string_lossy().to_string(),
                revision: Revision(1),
                created_at: Timestamp(1_000),
                updated_at: Timestamp(1_000),
                task_id: Some(TaskId::from(tid)),
                project_id: Some(pid.clone()),
                workflow_stage: wf,
                task_kind: TaskKind::Standard,
                workflow_digest: None,
                base_sha: Some(base_sha.to_string()),
                head_sha: None,
                plan_version: 1,
                policy_digest: Some(PolicyDigest::from("pol_p")),
                budget_policy: BudgetPolicy::default(),
                cost_summary: CostSummary::default(),
                repair_attempts: 0,
                pr_url: None,
            },
            ownership: RunOwnership {
                assigned_runner_id: Some(RunnerId::from(owner)),
                lease_epoch: LeaseEpoch(1),
                runner_unreachable: false,
            },
        });
    }
    let snap = data.snapshot.clone();
    drop(data);
    seed_local_sqlite(db_path, mock_url, repo_dir, &snap);
}

fn seed_local_sqlite(
    db_path: &Path,
    mock_url: &str,
    repo_dir: &Path,
    snap: &runlark_domain::WorkspaceSnapshot,
) {
    let wid = WorkspaceId::from("ws_pipe");
    let pid = ProjectId::from("prj_pipe");
    let mut conn = open_sqlite(db_path).unwrap();
    initialize_schema(&conn).unwrap();
    ProjectStore::create_project(
        &conn,
        &Project::new(
            pid.clone(),
            "Pipe",
            repo_dir.to_string_lossy().as_ref(),
            "main",
            BudgetPolicy::default(),
            PolicyDigest::from("pol_p"),
            Timestamp(1_000),
        ),
    )
    .unwrap();
    SyncStore::promote_project_to_team_atomic(
        &mut conn,
        &pid,
        mock_url,
        &ServerPromotionAck {
            project_id: pid.clone(),
            workspace_id: wid.clone(),
            ownership_epoch: 1,
            initial_cursor: CursorSeq(0),
            acknowledged_at: Timestamp(1_000),
        },
        Timestamp(1_000),
    )
    .unwrap();
    TeamAuthStore::save_session(
        &conn,
        &TeamAuthSession {
            server_url: mock_url.to_string(),
            user_id: UserId::from("usr_owner"),
            display_name: "Owner".to_string(),
            bearer_token: "tok".to_string(),
            default_workspace_id: Some(wid),
            runner_id: Some(RunnerId::from("runner_mine")),
            runner_name: Some("Mine".to_string()),
            updated_at: Timestamp(1_000),
        },
    )
    .unwrap();
    SyncStore::apply_snapshot_atomic(&mut conn, mock_url, snap, Timestamp(1_100)).unwrap();
    SessionStore::update_run_worktree(
        &conn,
        &RunId::from("run_verif"),
        repo_dir.to_string_lossy().as_ref(),
        Timestamp(1_100),
    )
    .unwrap();
}

async fn assert_reconnect_recovery_and_accept(
    state: AppState,
    db_path: &Path,
    mock_url: &str,
    pipe_mock: &PipelineMockState,
) {
    let mock_state = &pipe_mock.base;
    let reports = &pipe_mock.reports;
    // Once heartbeat transitions runner to Online, sync_workspace heartbeats FIRST, flushes buffer,
    // uploads CAS BEFORE AttachArtifact, and submits MoveTaskStage(Acceptance).
    pipe_mock.heartbeat_reonline.store(true, Ordering::SeqCst);
    let sync_summary = TeamSyncWorker::new(state)
        .sync_workspace(mock_url, "tok", &WorkspaceId::from("ws_pipe"))
        .await
        .unwrap();
    assert!(sync_summary.cas_uploaded >= 1);
    assert_eq!(sync_summary.outbox_acked, 2);

    let recorded = reports.lock().await;
    assert_eq!(recorded.len(), 2);
    assert_eq!(
        recorded[0].1.workflow_stage,
        Some(WorkflowStage::Verification)
    );
    assert_eq!(
        recorded[1].1.workflow_stage,
        Some(WorkflowStage::HumanAcceptance)
    );
    assert_eq!(recorded[1].1.status, Some(RunStatus::WaitingForApproval));
    assert!(recorded[1].1.head_sha.is_some());
    drop(recorded);

    let data = mock_state.inner.lock().await;
    assert!(data.commands_seen.iter().any(|c| matches!(
        c,
        TeamCommand::MoveTaskStage(m)
            if m.task_id == TaskId::from("tsk_verif") && m.target_stage == TaskStage::Acceptance
    )));
    assert!(
        data.commands_seen
            .iter()
            .any(|c| matches!(c, TeamCommand::AttachArtifact(a) if a.label == "evidence_bundle"))
    );
    drop(data);

    let (daemon_url, _s, _sd, daemon_handle) = spawn_daemon(db_path).await;
    let accept_status = Client::new()
        .post(format!("{daemon_url}/api/v1/tasks/tsk_verif/accept"))
        .json(&json!({ "expected_revision": 2 }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(accept_status, 200);
    daemon_handle.abort();
}

#[tokio::test]
async fn pipeline_reports_and_buffers_team_state_and_fencing() {
    let tmp = TempDir::new().unwrap();
    let repo_dir = tmp.path().join("repo");
    fs::create_dir_all(&repo_dir).unwrap();
    let base_sha = init_git_repo(&repo_dir);
    let db_path = tmp.path().join("runlark.db");
    let (pipe_mock, mock_url, mock_handle) = spawn_pipeline_mock().await;
    let mock_state = &pipe_mock.base;
    seed_state(&db_path, &mock_url, mock_state, &repo_dir, &base_sha).await;

    let verification = VerificationService::with_cas_dir(tmp.path().join("wasm-cas")).unwrap();
    let solo_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("workflows")
        .join("solo-default");
    verification.install_package_dir(&solo_dir).unwrap();
    let state = AppState::with_verification(db_path.clone(), verification);
    // Unfenced runner must fail BEFORE mutating local SQLite.
    let err = WorkflowPipeline::advance_stage_with_task(
        &state,
        &RunId::from("run_other"),
        WorkflowStage::Implementation,
        Timestamp(2_000),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, PipelineError::TeamFencing(_)));
    {
        let conn = open_sqlite(&db_path).unwrap();
        assert_eq!(
            SessionStore::get_run(&conn, &RunId::from("run_other"))
                .unwrap()
                .unwrap()
                .workflow_stage,
            WorkflowStage::Plan
        );
    }

    // Offline verify_and_advance buffers Verification + HumanAcceptance + MoveTaskStage(Acceptance).
    mock_state.offline.store(true, Ordering::SeqCst);
    let out = WorkflowPipeline::verify_and_advance(
        &state,
        &RunId::from("run_verif"),
        VerifyPipelineOptions::default(),
        Timestamp(3_000),
    )
    .await
    .unwrap();
    assert!(matches!(
        out,
        runlark_cli::VerifyPipelineOutcome::PassedForAcceptance { .. }
    ));

    {
        let conn = open_sqlite(&db_path).unwrap();
        assert_eq!(
            RunnerBufferStore::list_unflushed(&conn, Some(&WorkspaceId::from("ws_pipe")))
                .unwrap()
                .len(),
            2
        );
        let outbox =
            SyncOutboxStore::list_pending_for_workspace(&conn, &WorkspaceId::from("ws_pipe"))
                .unwrap();
        assert!(outbox.iter().any(|e| e.command_type == "move_task_stage"));
        assert!(outbox.iter().any(|e| e.command_type == "attach_artifact"));
    }

    // If runner stays Unreachable (`runner_not_online`), buffered events MUST remain unflushed!
    pipe_mock.runner_online.store(false, Ordering::SeqCst);
    pipe_mock.heartbeat_reonline.store(false, Ordering::SeqCst);
    mock_state.offline.store(false, Ordering::SeqCst);
    let (flushed, defer_conflicts) = RunnerAgentWorker::new(state.clone())
        .flush_buffered_events(&mock_url, "tok", &WorkspaceId::from("ws_pipe"))
        .await
        .unwrap();
    assert_eq!(flushed, 0);
    assert_eq!(defer_conflicts, 1);
    {
        let conn = open_sqlite(&db_path).unwrap();
        assert_eq!(
            RunnerBufferStore::list_unflushed(&conn, Some(&WorkspaceId::from("ws_pipe")))
                .unwrap()
                .len(),
            2
        );
    }

    assert_reconnect_recovery_and_accept(state, &db_path, &mock_url, &pipe_mock).await;
    mock_handle.abort();
}
