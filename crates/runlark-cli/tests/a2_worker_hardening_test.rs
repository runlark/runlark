use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use reqwest::Client;
use runlark_cli::{
    AppState, RunOwnershipResponse, RunnerAgentWorker, WorkflowPipeline, build_router,
};
use runlark_domain::{
    BudgetPolicy, CostSummary, CursorSeq, EventSeq, LeaseEpoch, PolicyDigest, Project, ProjectId,
    Revision, Run, RunId, RunOwnership, RunStatus, RunnerEventInput, RunnerId, RunnerRegistration,
    RunnerRunReport, RunnerStatus, ServerPromotionAck, Session, SessionId, SessionStatus, Task,
    TaskId, TaskKind, TaskStage, Timestamp, UserId, WorkflowStage, WorkspaceId,
};
use runlark_local::{
    DraftStore, ProjectStore, RunnerBufferStore, SessionStore, SyncStore, TaskStore,
    TeamAuthSession, TeamAuthStore, initialize_schema, open_sqlite,
};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

async fn spawn_test_daemon(db_path: &Path) -> (String, AppState, tokio::task::JoinHandle<()>) {
    let conn = open_sqlite(db_path).expect("open db");
    initialize_schema(&conn).expect("init schema");
    drop(conn);
    let state = AppState::new(db_path.to_path_buf());
    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (base_url, state, handle)
}

fn make_run(rid: &str, sid: &str, worktree: &str, kind: TaskKind) -> Run {
    Run {
        id: RunId::from(rid),
        session_id: SessionId::from(sid),
        status: RunStatus::Running,
        worktree_path: worktree.to_string(),
        revision: Revision(1),
        created_at: Timestamp(100),
        updated_at: Timestamp(100),
        task_id: None,
        project_id: None,
        workflow_stage: kind.initial_workflow_stage(),
        task_kind: kind,
        workflow_digest: None,
        base_sha: None,
        head_sha: None,
        plan_version: 0,
        policy_digest: None,
        budget_policy: BudgetPolicy::default(),
        cost_summary: CostSummary::default(),
        repair_attempts: 0,
        pr_url: None,
    }
}

fn make_session(sid: &str, tid: &TaskId, rid: &str) -> Session {
    Session {
        id: SessionId::from(sid),
        task_id: tid.clone(),
        cwd: "/tmp".to_string(),
        status: SessionStatus::Active,
        current_revision: Revision(1),
        active_run_id: Some(RunId::from(rid)),
        created_at: Timestamp(100),
        updated_at: Timestamp(100),
    }
}

fn seed_runner_buffer_fixture(db_path: &Path) {
    let conn = open_sqlite(db_path).unwrap();
    SessionStore::create_session(
        &conn,
        &make_session("sess_c", &TaskId::from("tsk_c"), "run_c"),
    )
    .unwrap();
    let mut run = make_run("run_c", "sess_c", "", TaskKind::QuickFix);
    run.status = RunStatus::CancelRequested;
    run.revision = Revision(2);
    SessionStore::save_run(&conn, &run).unwrap();

    for (seq, status, stage, event_type) in [
        (
            1,
            None,
            Some(WorkflowStage::Verification),
            "run_stage_verification",
        ),
        (2, Some(RunStatus::Failed), None, "run_budget_halted"),
    ] {
        RunnerBufferStore::buffer_report(
            &conn,
            &WorkspaceId::from("ws_rnr"),
            &RunId::from("run_c"),
            &RunnerRunReport {
                runner_id: RunnerId::from("rnr_pre_snap"),
                lease_epoch: LeaseEpoch(1),
                status,
                workflow_stage: stage,
                head_sha: None,
                base_sha: None,
                events: vec![RunnerEventInput {
                    seq: EventSeq(seq),
                    event_type: event_type.to_string(),
                    payload_json: "{}".to_string(),
                    created_at: Timestamp(100 + seq),
                }],
            },
        )
        .unwrap();
    }
}

#[tokio::test]
async fn runner_register_before_snapshot_and_transition_conflict_does_not_drop_next_chunk() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("runlark.db");
    let (daemon_url, state, daemon_handle) = spawn_test_daemon(&db_path).await;

    let second_chunk_seen = Arc::new(AtomicUsize::new(0));
    let seen_clone = Arc::clone(&second_chunk_seen);
    let srv_app = Router::new()
        .route(
            "/api/v1/auth/whoami",
            get(|| async { Json(json!({ "user_id": "usr_owner", "display_name": "Owner" })) }),
        )
        .route(
            "/api/v1/workspaces/{wid}/runners/register",
            post(|| async {
                Json(RunnerRegistration {
                    id: RunnerId::from("rnr_pre_snap"),
                    workspace_id: WorkspaceId::from("ws_rnr"),
                    owner_user_id: UserId::from("usr_owner"),
                    name: "Laptop".to_string(),
                    status: RunnerStatus::Online,
                    last_heartbeat_at: Timestamp(1_000),
                    created_at: Timestamp(1_000),
                })
            }),
        )
        .route(
            "/api/v1/workspaces/{wid}/runs/{rid}/runner-state",
            post(move |Json(report): Json<RunnerRunReport>| {
                let seen = Arc::clone(&seen_clone);
                async move {
                    if report.workflow_stage == Some(WorkflowStage::Verification) {
                        return Err((
                            StatusCode::CONFLICT,
                            Json(json!({
                                "error": "transition_error",
                                "message": "run is cancel_requested"
                            })),
                        ));
                    }
                    seen.fetch_add(1, Ordering::SeqCst);
                    let mut run = make_run("run_c", "sess_c", "", TaskKind::QuickFix);
                    run.status = RunStatus::Failed;
                    run.revision = Revision(3);
                    Ok(Json(RunOwnershipResponse {
                        run,
                        ownership: RunOwnership {
                            assigned_runner_id: Some(RunnerId::from("rnr_pre_snap")),
                            lease_epoch: LeaseEpoch(1),
                            runner_unreachable: false,
                        },
                    }))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let srv_url = format!("http://{}", listener.local_addr().unwrap());
    let srv_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, srv_app).await;
    });

    let client = Client::new();
    assert_eq!(
        client
            .post(format!("{daemon_url}/api/v1/team/login"))
            .json(&json!({ "server_url": srv_url, "bearer_token": "tok_r" }))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let reg = client
        .post(format!("{daemon_url}/api/v1/team/runners/register"))
        .json(&json!({ "workspace_id": "ws_rnr", "name": "Laptop" }))
        .send()
        .await
        .unwrap();
    assert_eq!(reg.status(), 200);
    let auth_after = TeamAuthStore::get_by_server(&open_sqlite(&db_path).unwrap(), &srv_url)
        .unwrap()
        .unwrap();
    assert_eq!(auth_after.runner_id, Some(RunnerId::from("rnr_pre_snap")));
    assert_eq!(
        auth_after.default_workspace_id,
        Some(WorkspaceId::from("ws_rnr"))
    );

    seed_runner_buffer_fixture(&db_path);
    let (flushed, conflicts) = RunnerAgentWorker::new(state)
        .flush_buffered_events(&srv_url, "tok_r", &WorkspaceId::from("ws_rnr"))
        .await
        .unwrap();
    assert_eq!(conflicts, 1);
    assert_eq!(flushed, 1);
    assert_eq!(second_chunk_seen.load(Ordering::SeqCst), 1);

    daemon_handle.abort();
    srv_handle.abort();
}

fn seed_rev_project_fixture(db_path: &Path, srv_url: &str) {
    let mut conn = open_sqlite(db_path).unwrap();
    let proj = Project::new(
        ProjectId::from("prj_rev"),
        "Rev",
        "/tmp",
        "main",
        BudgetPolicy::default(),
        PolicyDigest::from("pol"),
        Timestamp(100),
    );
    ProjectStore::create_project(&conn, &proj).unwrap();
    SyncStore::promote_project_to_team_atomic(
        &mut conn,
        &proj.id,
        srv_url,
        &ServerPromotionAck {
            workspace_id: WorkspaceId::from("ws_rev"),
            project_id: proj.id.clone(),
            ownership_epoch: 1,
            initial_cursor: CursorSeq(0),
            acknowledged_at: Timestamp(100),
        },
        Timestamp(100),
    )
    .unwrap();
    TeamAuthStore::save_session(
        &conn,
        &TeamAuthSession {
            server_url: srv_url.to_string(),
            user_id: UserId::from("usr_rev"),
            display_name: "Alice".to_string(),
            bearer_token: "tok_rev".to_string(),
            default_workspace_id: Some(WorkspaceId::from("ws_rev")),
            runner_id: Some(RunnerId::from("rnr_rev")),
            runner_name: Some("Rev".to_string()),
            updated_at: Timestamp(100),
        },
    )
    .unwrap();
    let task = Task {
        id: TaskId::from("tsk_rev"),
        project_id: proj.id.clone(),
        title: "Std".to_string(),
        description: "desc".to_string(),
        kind: TaskKind::Standard,
        stage: TaskStage::InProgress,
        workflow_stage: WorkflowStage::Clarification,
        plan_text: None,
        plan_version: 0,
        base_ref: None,
        head_ref: None,
        pr_url: None,
        active_session_id: None,
        active_run_id: None,
        latest_run_id: None,
        budget_override: None,
        revision: Revision(4),
        created_at: Timestamp(100),
        updated_at: Timestamp(100),
    };
    TaskStore::create_task(&conn, &task).unwrap();
    SessionStore::create_session(&conn, &make_session("sess_rev", &task.id, "run_rev")).unwrap();
    let mut run = make_run("run_rev", "sess_rev", "/tmp", TaskKind::Standard);
    run.project_id = Some(proj.id);
    run.task_id = Some(task.id);
    SessionStore::save_run(&conn, &run).unwrap();
    conn.execute(
        "INSERT INTO run_ownership (run_id, assigned_runner_id, lease_epoch, runner_unreachable, updated_at) VALUES ('run_rev', 'rnr_rev', 1, 0, 100)",
        [],
    )
    .unwrap();
}

#[tokio::test]
async fn team_stage_advance_preserves_task_revision_and_post_commit_comment_does_not_duplicate() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("runlark.db");
    let (daemon_url, state, daemon_handle) = spawn_test_daemon(&db_path).await;

    let srv_app = Router::new()
        .route(
            "/api/v1/workspaces/{wid}/commands",
            post(|| async {
                Json(json!({
                    "command_id": "cmd_cmt_fixed",
                    "idempotent_replay": false,
                    "cursor_seq": 2,
                    "aggregate_type": "comment",
                    "aggregate_id": "cmt_any",
                    "revision": 1
                }))
            }),
        )
        .route(
            "/api/v1/workspaces/{wid}/sync/snapshot",
            get(|| async {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({ "error": "down", "message": "sync down after commit" })),
                )
            }),
        )
        .route(
            "/api/v1/workspaces/{wid}/runs/{rid}/runner-state",
            post(|| async {
                let mut run = make_run("run_rev", "sess_rev", "", TaskKind::Standard);
                run.workflow_stage = WorkflowStage::Plan;
                run.revision = Revision(2);
                Json(RunOwnershipResponse {
                    run,
                    ownership: RunOwnership {
                        assigned_runner_id: Some(RunnerId::from("rnr_rev")),
                        lease_epoch: LeaseEpoch(1),
                        runner_unreachable: false,
                    },
                })
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let srv_url = format!("http://{}", listener.local_addr().unwrap());
    let srv_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, srv_app).await;
    });
    seed_rev_project_fixture(&db_path, &srv_url);

    WorkflowPipeline::advance_stage_with_task(
        &state,
        &RunId::from("run_rev"),
        WorkflowStage::Plan,
        Timestamp(200),
    )
    .await
    .unwrap();
    let task_after = TaskStore::get_task(&open_sqlite(&db_path).unwrap(), &TaskId::from("tsk_rev"))
        .unwrap()
        .unwrap();
    assert_eq!(task_after.workflow_stage, WorkflowStage::Plan);
    assert_eq!(task_after.revision, Revision(4));

    let first_resp = Client::new()
        .post(format!("{daemon_url}/api/v1/tasks/tsk_rev/comments"))
        .json(&json!({ "command_id": "cmd_cmt_fixed", "body": "looks good" }))
        .send()
        .await
        .unwrap();
    assert_eq!(first_resp.status(), 201);
    let first_body: Value = first_resp.json().await.unwrap();
    assert_eq!(first_body["delivery"], "synced");

    let replay_resp = Client::new()
        .post(format!("{daemon_url}/api/v1/tasks/tsk_rev/comments"))
        .json(&json!({ "command_id": "cmd_cmt_fixed", "body": "looks good" }))
        .send()
        .await
        .unwrap();
    assert_eq!(replay_resp.status(), 201);
    let replay_body: Value = replay_resp.json().await.unwrap();
    assert_eq!(replay_body["comment"]["id"], first_body["comment"]["id"]);
    assert!(
        DraftStore::list_drafts(&open_sqlite(&db_path).unwrap(), None)
            .unwrap()
            .is_empty()
    );

    daemon_handle.abort();
    srv_handle.abort();
}
