mod a2_team_mock;

use a2_team_mock::{
    MockTeamServerState, check_auth, mock_commands, mock_events, mock_snapshot, mock_whoami,
    spawn_daemon,
};
use axum::body::Bytes;
use axum::extract::{Extension, Path as AxumPath, State as AxumState};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use reqwest::Client;
use runlark_cli::{AppState, TeamSyncWorker};
use runlark_domain::{
    ArtifactDigest, ArtifactKind, ArtifactRef, BudgetPolicy, CostSummary, CursorSeq, Decision,
    DraftSyncState, PolicyDigest, Project, ProjectId, ProjectOwnershipMode, Revision, Run, RunId,
    RunOwnership, RunStatus, ServerPromotionAck, Session, SessionId, SessionStatus, Task,
    TaskComment, TaskId, TaskKind, TaskStage, TeamProjectSpec, TeamRunEntry, Timestamp,
    WorkflowStage,
};
use runlark_local::{
    ArtifactCasStore, ProjectStore, SessionStore, SyncOutboxStatus, TaskStore, open_sqlite,
    sha256_hex,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tempfile::TempDir;
use tokio::sync::Mutex;

type BlobMap = Arc<Mutex<HashMap<String, Vec<u8>>>>;

async fn mock_promote(
    AxumState(st): AxumState<MockTeamServerState>,
    headers: HeaderMap,
    AxumPath(wid): AxumPath<String>,
    Json(body): Json<Value>,
) -> Result<Json<ServerPromotionAck>, (StatusCode, Json<Value>)> {
    check_auth(&headers, &st)?;
    let mut data = st.inner.lock().await;
    let project: TeamProjectSpec = serde_json::from_value(body["project"].clone()).unwrap();
    let tasks: Vec<Task> = serde_json::from_value(body["tasks"].clone()).unwrap();
    let runs: Vec<Run> = serde_json::from_value(body["runs"].clone()).unwrap();
    let decisions: Vec<Decision> = serde_json::from_value(body["decisions"].clone()).unwrap();
    let project_id = project.project_id.clone();

    data.snapshot.projects.push(project);
    data.snapshot.tasks.extend(tasks);
    for r in runs {
        data.snapshot.runs.push(TeamRunEntry {
            run: r,
            ownership: RunOwnership::default(),
        });
    }
    data.snapshot.decisions.extend(decisions);
    data.cursor += 1;
    data.snapshot.cursor_seq = CursorSeq(data.cursor);

    Ok(Json(ServerPromotionAck {
        project_id,
        workspace_id: runlark_domain::WorkspaceId::from(wid),
        ownership_epoch: body["ownership_epoch"].as_u64().unwrap_or(1),
        initial_cursor: CursorSeq(data.cursor),
        acknowledged_at: Timestamp(1_000_000),
    }))
}

async fn mock_put_cas(
    AxumState(st): AxumState<MockTeamServerState>,
    Extension(blobs): Extension<BlobMap>,
    headers: HeaderMap,
    AxumPath((_wid, digest)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<ArtifactRef>, (StatusCode, Json<Value>)> {
    check_auth(&headers, &st)?;
    blobs.lock().await.insert(digest.clone(), body.to_vec());
    Ok(Json(ArtifactRef {
        digest_sha256: ArtifactDigest::parse(digest).unwrap(),
        kind: ArtifactKind::EvidenceBundle,
        size_bytes: body.len() as u64,
        media_type: "text/plain".to_string(),
        created_at: Timestamp(5_000),
    }))
}

async fn mock_get_cas(
    AxumState(st): AxumState<MockTeamServerState>,
    Extension(blobs): Extension<BlobMap>,
    headers: HeaderMap,
    AxumPath((_wid, digest)): AxumPath<(String, String)>,
) -> Result<Vec<u8>, (StatusCode, Json<Value>)> {
    check_auth(&headers, &st)?;
    blobs.lock().await.get(&digest).cloned().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "blob missing" })),
        )
    })
}

fn init_git_repo(dir: &Path) {
    for args in [
        &["init"][..],
        &["config", "user.email", "test@runlark.dev"],
        &["config", "user.name", "Runlark Test"],
    ] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    fs::write(dir.join("README.md"), "# Team Sync Test\n").unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["add", "README.md"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["commit", "-m", "init"])
            .output()
            .unwrap()
            .status
            .success()
    );
}

async fn spawn_mock_team_server() -> (
    String,
    MockTeamServerState,
    BlobMap,
    tokio::task::JoinHandle<()>,
) {
    let state = MockTeamServerState::new("ws_team_1", "Team WS", "team-ws", Some("tok_secret"));
    let blobs: BlobMap = Arc::new(Mutex::new(HashMap::new()));
    let router = Router::new()
        .route("/api/v1/auth/whoami", get(mock_whoami))
        .route(
            "/api/v1/workspaces/{wid}/projects/promote",
            post(mock_promote),
        )
        .route("/api/v1/workspaces/{wid}/sync/snapshot", get(mock_snapshot))
        .route("/api/v1/workspaces/{wid}/sync/events", get(mock_events))
        .route("/api/v1/workspaces/{wid}/commands", post(mock_commands))
        .route(
            "/api/v1/workspaces/{wid}/artifacts/{digest}",
            put(mock_put_cas).get(mock_get_cas),
        )
        .layer(Extension(blobs.clone()))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{addr}"), state, blobs, handle)
}

fn seed_project_and_tasks(state: &AppState, repo_dir: &Path) {
    let conn = open_sqlite(&state.db_path).unwrap();
    let project = Project::new(
        ProjectId::from("prj_1"),
        "Sync Repo",
        repo_dir.to_string_lossy().as_ref(),
        "main",
        BudgetPolicy::default(),
        PolicyDigest::from("pol_1"),
        Timestamp(1_000),
    );
    ProjectStore::create_project(&conn, &project).unwrap();
    for (tid, stage, active_run) in [
        ("tsk_ready", TaskStage::Ready, None),
        ("tsk_acc", TaskStage::Acceptance, Some("run_active")),
    ] {
        TaskStore::create_task(
            &conn,
            &Task {
                id: TaskId::from(tid),
                project_id: ProjectId::from("prj_1"),
                title: format!("Task {tid}"),
                description: "sync me".to_string(),
                kind: TaskKind::Standard,
                stage,
                workflow_stage: WorkflowStage::Plan,
                plan_text: Some("Initial plan".to_string()),
                plan_version: 1,
                base_ref: Some("main".to_string()),
                head_ref: None,
                pr_url: None,
                active_session_id: None,
                active_run_id: None,
                latest_run_id: active_run.map(RunId::from),
                budget_override: None,
                revision: Revision(1),
                created_at: Timestamp(1_000),
                updated_at: Timestamp(1_000),
            },
        )
        .unwrap();
    }
    SessionStore::create_session(
        &conn,
        &Session {
            id: SessionId::from("ses_active"),
            task_id: TaskId::from("tsk_acc"),
            cwd: repo_dir.to_string_lossy().to_string(),
            status: SessionStatus::Active,
            current_revision: Revision(1),
            active_run_id: Some(RunId::from("run_active")),
            created_at: Timestamp(1_000),
            updated_at: Timestamp(1_000),
        },
    )
    .unwrap();
    SessionStore::save_run(
        &conn,
        &Run {
            id: RunId::from("run_active"),
            session_id: SessionId::from("ses_active"),
            status: RunStatus::Running,
            worktree_path: repo_dir.to_string_lossy().to_string(),
            revision: Revision(1),
            created_at: Timestamp(1_000),
            updated_at: Timestamp(1_000),
            task_id: Some(TaskId::from("tsk_acc")),
            project_id: Some(ProjectId::from("prj_1")),
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
        },
    )
    .unwrap();
}

async fn phase_login_and_promote(http: &Client, daemon_url: &str, mock_url: &str, db_path: &Path) {
    let login_resp = http
        .post(format!("{daemon_url}/api/v1/team/login"))
        .json(&json!({
            "server_url": mock_url,
            "bearer_token": "tok_secret",
            "workspace_id": "ws_team_1"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(login_resp.status(), 200);

    let blocked = http
        .post(format!("{daemon_url}/api/v1/projects/prj_1/promote-team"))
        .json(&json!({ "workspace_id": "ws_team_1" }))
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), 409);

    {
        let conn = open_sqlite(db_path).unwrap();
        let mut run = SessionStore::get_run(&conn, &RunId::from("run_active"))
            .unwrap()
            .unwrap();
        run.status = RunStatus::Completed;
        run.revision = Revision(2);
        SessionStore::save_run(&conn, &run).unwrap();
        let cas = ArtifactCasStore::from_data_dir(db_path.parent().unwrap());
        let art = cas
            .put_bytes(
                &conn,
                ArtifactKind::EvidenceBundle,
                "text/plain",
                b"team-evidence-1",
                Timestamp(1_200),
            )
            .unwrap();
        ArtifactCasStore::link_to_run(
            &conn,
            &RunId::from("run_active"),
            &art.digest_sha256,
            "evidence",
            Timestamp(1_200),
        )
        .unwrap();
    }

    let ok_promote = http
        .post(format!("{daemon_url}/api/v1/projects/prj_1/promote-team"))
        .json(&json!({ "workspace_id": "ws_team_1" }))
        .send()
        .await
        .unwrap();
    assert_eq!(ok_promote.status(), 200);
    let promoted: Value = ok_promote.json().await.unwrap();
    assert!(matches!(
        serde_json::from_value::<ProjectOwnershipMode>(
            promoted["binding"]["ownership_mode"].clone()
        )
        .unwrap(),
        ProjectOwnershipMode::TeamManaged { .. }
    ));
}

async fn phase_verify_offline_gates(http: &Client, daemon_url: &str, db_path: &Path) {
    for (path, body) in [
        (
            "/api/v1/tasks/tsk_ready/clarify",
            json!({ "clarification": "offline clarify blocked", "advance_to_plan": true }),
        ),
        (
            "/api/v1/tasks/tsk_ready/plan",
            json!({ "plan_text": "offline plan blocked", "expected_revision": 1 }),
        ),
        (
            "/api/v1/tasks/tsk_ready/start",
            json!({ "expected_revision": 1 }),
        ),
        (
            "/api/v1/tasks/tsk_ready/move",
            json!({ "stage": "in_progress", "expected_revision": 1 }),
        ),
        (
            "/api/v1/tasks/tsk_acc/accept",
            json!({ "expected_revision": 1 }),
        ),
        (
            "/api/v1/tasks/tsk_acc/reject",
            json!({ "reason": "needs work", "expected_revision": 1 }),
        ),
        (
            "/api/v1/tasks/tsk_acc/pr",
            json!({ "run_id": "run_active" }),
        ),
        ("/api/v1/chat/dispatch", json!({ "input": "/pr tsk_acc" })),
        (
            "/api/v1/chat/dispatch",
            json!({ "input": "/pr run_active" }),
        ),
        (
            "/api/v1/chat/dispatch",
            json!({ "input": "/stop run_active offline stop" }),
        ),
    ] {
        let resp = http
            .post(format!("{daemon_url}{path}"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 503, "expected 503 offline gate for {path}");
        let err: Value = resp.json().await.unwrap();
        assert_eq!(err["error"], "team_server_offline");
    }
    let conn = open_sqlite(db_path).unwrap();
    let ready = TaskStore::get_task(&conn, &TaskId::from("tsk_ready"))
        .unwrap()
        .unwrap();
    assert_eq!(ready.stage, TaskStage::Ready);
    assert_eq!(ready.revision, Revision(1));
    assert_eq!(
        ProjectStore::list_runs_for_project(&conn, &ProjectId::from("prj_1"))
            .unwrap()
            .len(),
        1
    );
    let run_after = SessionStore::get_run(&conn, &RunId::from("run_active"))
        .unwrap()
        .unwrap();
    assert_eq!(run_after.status, RunStatus::Completed);
}

async fn phase_offline_drafts_and_conflict(
    http: &Client,
    daemon_url: &str,
    db_path: &Path,
    mock_state: &MockTeamServerState,
) {
    {
        let conn = open_sqlite(db_path).unwrap();
        let cas = ArtifactCasStore::from_data_dir(db_path.parent().unwrap());
        let art2 = cas
            .put_bytes(
                &conn,
                ArtifactKind::Diff,
                "text/plain",
                b"team-evidence-2",
                Timestamp(1_500),
            )
            .unwrap();
        ArtifactCasStore::link_to_run(
            &conn,
            &RunId::from("run_active"),
            &art2.digest_sha256,
            "diff",
            Timestamp(1_500),
        )
        .unwrap();
    }
    let cmt_resp = http
        .post(format!("{daemon_url}/api/v1/tasks/tsk_ready/comments"))
        .json(&json!({ "body": "offline comment queued" }))
        .send()
        .await
        .unwrap();
    assert_eq!(cmt_resp.status(), 202);
    assert_eq!(
        cmt_resp.json::<Value>().await.unwrap()["delivery"],
        "queued_offline"
    );

    let create_draft_resp = http
        .post(format!("{daemon_url}/api/v1/team/drafts"))
        .json(&json!({
            "draft_id": "draft_plan_conflict",
            "project_id": "prj_1",
            "task_id": "tsk_ready",
            "kind": "plan_edit",
            "payload_json": json!({ "plan_text": "Edited while offline" }).to_string(),
            "base_revision": 1,
            "queue_sync": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_draft_resp.status(), 201);
    mock_state.inner.lock().await.current_task_rev = 3;
}

async fn phase_verify_gap_and_cas(
    state: AppState,
    mock_url: &str,
    mock_state: &MockTeamServerState,
    blobs: &BlobMap,
) {
    mock_state.inner.lock().await.emit_gap_once = true;
    let worker = TeamSyncWorker::new(state);
    let summary = worker
        .sync_workspace(
            mock_url,
            "tok_secret",
            &runlark_domain::WorkspaceId::from("ws_team_1"),
        )
        .await
        .unwrap();
    assert_eq!(summary.snapshots_applied, 1);

    let expected_digest = sha256_hex(b"team-evidence-1");
    let expected_digest_2 = sha256_hex(b"team-evidence-2");
    {
        let blob_map = blobs.lock().await;
        assert!(blob_map.contains_key(&expected_digest));
        assert!(blob_map.contains_key(&expected_digest_2));
    }
    let downloaded = worker
        .download_artifact(
            mock_url,
            "tok_secret",
            &runlark_domain::WorkspaceId::from("ws_team_1"),
            &ArtifactDigest::parse(expected_digest).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(downloaded, b"team-evidence-1");
    assert!(!mock_state.inner.lock().await.commands_seen.is_empty());
}

#[tokio::test]
async fn promotion_offline_gate_conflict_drafts_gap_fallback_and_cas_work_end_to_end() {
    let tmp = TempDir::new().unwrap();
    let repo_dir = tmp.path().join("repo");
    fs::create_dir_all(&repo_dir).unwrap();
    init_git_repo(&repo_dir);
    let db_path = tmp.path().join("runlark.db");

    let (mock_url, mock_state, blobs, mock_handle) = spawn_mock_team_server().await;
    let (daemon_url, state, _shutdown, daemon_handle) = spawn_daemon(&db_path).await;
    seed_project_and_tasks(&state, &repo_dir);
    let http = Client::new();

    phase_login_and_promote(&http, &daemon_url, &mock_url, &db_path).await;

    mock_state.offline.store(true, Ordering::SeqCst);
    phase_verify_offline_gates(&http, &daemon_url, &db_path).await;
    phase_offline_drafts_and_conflict(&http, &daemon_url, &db_path, &mock_state).await;

    mock_state.offline.store(false, Ordering::SeqCst);
    let sync_json: Value = http
        .post(format!("{daemon_url}/api/v1/team/sync"))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(sync_json["outbox_acked"].as_u64().unwrap() >= 1);
    assert_eq!(sync_json["outbox_conflicts"], 1);
    assert_eq!(sync_json["cas_uploaded"], 1);

    let list_payload: Value = http
        .get(format!("{daemon_url}/api/v1/team/drafts"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let conflict_draft = list_payload["drafts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == "draft_plan_conflict")
        .unwrap();
    let sync_state: DraftSyncState =
        serde_json::from_value(conflict_draft["sync_state"].clone()).unwrap();
    assert_eq!(
        sync_state,
        DraftSyncState::Conflict {
            server_revision: Some(Revision(3)),
            reason: "plan base_revision mismatch".to_string()
        }
    );
    let conflict_outbox = list_payload["outbox"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["draft_id"] == "draft_plan_conflict")
        .unwrap();
    assert_eq!(
        conflict_outbox["status"],
        serde_json::to_value(SyncOutboxStatus::Conflict).unwrap()
    );

    let retry_resp = http
        .post(format!(
            "{daemon_url}/api/v1/team/drafts/draft_plan_conflict/retry"
        ))
        .json(&json!({ "expected_revision": 3 }))
        .send()
        .await
        .unwrap();
    assert_eq!(retry_resp.status(), 200);
    let comments: Vec<TaskComment> = http
        .get(format!("{daemon_url}/api/v1/tasks/tsk_ready/comments"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(comments.iter().any(|c| c.body == "offline comment queued"));

    phase_verify_gap_and_cas(state, &mock_url, &mock_state, &blobs).await;

    daemon_handle.abort();
    mock_handle.abort();
}
