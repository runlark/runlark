use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use reqwest::Client;
use runlark_cli::{AppState, build_router};
use runlark_domain::{
    BudgetPolicy, CommandId, CostSummary, CursorSeq, PolicyDigest, Project, ProjectId, Revision,
    Run, RunId, RunStatus, SendPromptCommand, ServerPromotionAck, Session, SessionCommand,
    SessionId, SessionStatus, Task, TaskId, TaskKind, TaskStage, Timestamp, UserId, WorkflowStage,
    WorkspaceId, WorkspaceMember, WorkspaceRole,
};
use runlark_local::{
    ProjectStore, SessionStore, SyncStore, TaskStore, TeamAuthSession, TeamAuthStore,
    initialize_schema, open_sqlite,
};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

async fn spawn_test_daemon(db_path: &Path) -> (String, tokio::task::JoinHandle<()>) {
    let conn = open_sqlite(db_path).expect("open db");
    initialize_schema(&conn).expect("init schema");
    drop(conn);
    let state = AppState::new(db_path.to_path_buf());
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (base_url, handle)
}

fn make_task(tid: &str, pid: &ProjectId, title: &str) -> Task {
    Task {
        id: TaskId::from(tid),
        project_id: pid.clone(),
        title: title.to_string(),
        description: "desc".to_string(),
        kind: TaskKind::QuickFix,
        stage: TaskStage::Inbox,
        workflow_stage: WorkflowStage::Implementation,
        plan_text: None,
        plan_version: 0,
        base_ref: None,
        head_ref: None,
        pr_url: None,
        active_session_id: None,
        active_run_id: None,
        latest_run_id: None,
        budget_override: None,
        revision: Revision(1),
        created_at: Timestamp(100),
        updated_at: Timestamp(100),
    }
}

fn make_run(rid: &str, sid: &str, pid: ProjectId, tid: TaskId) -> Run {
    Run {
        id: RunId::from(rid),
        session_id: SessionId::from(sid),
        status: RunStatus::Running,
        worktree_path: "/tmp".to_string(),
        revision: Revision(1),
        created_at: Timestamp(100),
        updated_at: Timestamp(100),
        task_id: Some(tid),
        project_id: Some(pid),
        workflow_stage: WorkflowStage::Implementation,
        task_kind: TaskKind::QuickFix,
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

#[tokio::test]
async fn daemon_outbound_routes_reject_path_traversal_before_calling_server() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("runlark.db");
    let (daemon_url, daemon_handle) = spawn_test_daemon(&db_path).await;

    let hits = Arc::new(AtomicUsize::new(0));
    let hits_clone = Arc::clone(&hits);
    let trap_app = Router::new().fallback(move || {
        let hits = Arc::clone(&hits_clone);
        async move {
            hits.fetch_add(1, Ordering::SeqCst);
            StatusCode::OK
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let trap_url = format!("http://{}", listener.local_addr().unwrap());
    let trap_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, trap_app).await;
    });

    {
        let conn = open_sqlite(&db_path).unwrap();
        TeamAuthStore::save_session(
            &conn,
            &TeamAuthSession {
                server_url: trap_url.clone(),
                user_id: UserId::from("usr_1"),
                display_name: "Alice".to_string(),
                bearer_token: "secret_tok".to_string(),
                default_workspace_id: Some(WorkspaceId::from("ws_ok")),
                runner_id: None,
                runner_name: None,
                updated_at: Timestamp(100),
            },
        )
        .unwrap();
        ProjectStore::create_project(
            &conn,
            &Project::new(
                ProjectId::from("prj_p"),
                "P",
                "/tmp",
                "main",
                BudgetPolicy::default(),
                PolicyDigest::from("pol"),
                Timestamp(100),
            ),
        )
        .unwrap();
    }

    let client = Client::new();
    let cases = [
        client
            .post(format!("{daemon_url}/api/v1/projects/prj_p/promote-team"))
            .json(&json!({ "workspace_id": "../evil", "server_url": trap_url })),
        client.get(format!("{daemon_url}/api/v1/team/workspaces/..%2Fevil")),
        client
            .patch(format!(
                "{daemon_url}/api/v1/team/workspaces/ws_ok/members/..%2Fadmin"
            ))
            .json(&json!({ "role": "owner" })),
        client
            .post(format!("{daemon_url}/api/v1/team/runs/..%2Fother/assign"))
            .json(&json!({ "workspace_id": "ws_ok", "runner_id": "rnr_1" })),
        client.get(format!(
            "{daemon_url}/api/v1/team/runners?workspace_id=ws%3Fx%3D1"
        )),
    ];
    for req in cases {
        assert_eq!(req.send().await.unwrap().status(), 400);
    }
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "invalid segments must never reach runlark-server"
    );
    daemon_handle.abort();
    trap_handle.abort();
}

fn seed_solo_and_team_fixture(db_path: &Path) {
    let mut conn = open_sqlite(db_path).unwrap();
    let solo_proj = Project::new(
        ProjectId::from("prj_solo"),
        "Solo",
        "/tmp",
        "main",
        BudgetPolicy::default(),
        PolicyDigest::from("pol_s"),
        Timestamp(100),
    );
    let team_proj = Project::new(
        ProjectId::from("prj_team"),
        "Team",
        "/tmp",
        "main",
        BudgetPolicy::default(),
        PolicyDigest::from("pol_t"),
        Timestamp(100),
    );
    ProjectStore::create_project(&conn, &solo_proj).unwrap();
    ProjectStore::create_project(&conn, &team_proj).unwrap();
    SyncStore::promote_project_to_team_atomic(
        &mut conn,
        &team_proj.id,
        "http://127.0.0.1:9",
        &ServerPromotionAck {
            workspace_id: WorkspaceId::from("ws_t"),
            project_id: team_proj.id.clone(),
            ownership_epoch: 1,
            initial_cursor: CursorSeq(0),
            acknowledged_at: Timestamp(100),
        },
        Timestamp(100),
    )
    .unwrap();

    let solo_task = make_task("tsk_solo", &solo_proj.id, "Solo");
    let team_task = make_task("tsk_team", &team_proj.id, "Team");
    TaskStore::create_task(&conn, &solo_task).unwrap();
    TaskStore::create_task(&conn, &team_task).unwrap();
    SessionStore::create_session(&conn, &make_session("sess_solo", &solo_task.id, "run_solo"))
        .unwrap();
    SessionStore::create_session(&conn, &make_session("sess_team", &team_task.id, "run_team"))
        .unwrap();
    SessionStore::save_run(
        &conn,
        &make_run("run_solo", "sess_solo", solo_proj.id, solo_task.id),
    )
    .unwrap();
    SessionStore::save_run(
        &conn,
        &make_run("run_team", "sess_team", team_proj.id, team_task.id),
    )
    .unwrap();
}

#[tokio::test]
async fn offline_gates_reject_mismatched_run_id_on_commands_and_pr() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("runlark.db");
    let (daemon_url, daemon_handle) = spawn_test_daemon(&db_path).await;
    seed_solo_and_team_fixture(&db_path);

    let client = Client::new();
    let bad_cmd = SessionCommand::SendPrompt(SendPromptCommand {
        command_id: CommandId::from("cmd_bypass_1"),
        session_id: SessionId::from("sess_team"),
        run_id: RunId::from("run_solo"),
        prompt: "hi".to_string(),
        expected_revision: Revision(1),
        created_at: Timestamp(200),
    });
    assert_eq!(
        client
            .post(format!("{daemon_url}/api/v1/commands"))
            .json(&bad_cmd)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );

    assert_eq!(
        client
            .post(format!("{daemon_url}/api/v1/tasks/tsk_solo/pr"))
            .json(&json!({ "run_id": "run_team" }))
            .send()
            .await
            .unwrap()
            .status(),
        409
    );

    daemon_handle.abort();
}

#[tokio::test]
async fn accept_invitation_preserves_one_time_token_when_initial_sync_fails() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("runlark.db");
    let (daemon_url, daemon_handle) = spawn_test_daemon(&db_path).await;

    let app = Router::new()
        .route(
            "/api/v1/invitations/accept",
            post(|| async {
                Json(json!({
                    "member": WorkspaceMember {
                        workspace_id: WorkspaceId::from("ws_inv"),
                        user_id: UserId::from("usr_bob"),
                        display_name: "Bob".to_string(),
                        role: WorkspaceRole::Member,
                        joined_at: Timestamp(1_000),
                    },
                    "bearer_token": "one_time_issued_bearer_token"
                }))
            }),
        )
        .route(
            "/api/v1/workspaces/{wid}/sync/snapshot",
            get(|| async {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({ "error": "transient", "message": "sync temporarily down" })),
                )
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let srv_url = format!("http://{}", listener.local_addr().unwrap());
    let srv_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let resp = Client::new()
        .post(format!("{daemon_url}/api/v1/team/invitations/accept"))
        .json(&json!({
            "server_url": srv_url,
            "invite_token": "inv_once",
            "display_name": "Bob"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["token_saved"], true);

    let saved = TeamAuthStore::get_by_server(&open_sqlite(&db_path).unwrap(), &srv_url)
        .unwrap()
        .expect("one-time bearer token must survive follow-up sync failure");
    assert_eq!(saved.bearer_token, "one_time_issued_bearer_token");

    daemon_handle.abort();
    srv_handle.abort();
}
