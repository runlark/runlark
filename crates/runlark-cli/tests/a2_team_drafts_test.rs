mod a2_team_mock;

use a2_team_mock::{
    MockTeamServerState, mock_commands, mock_events, mock_snapshot, mock_whoami, spawn_daemon,
};
use axum::Router;
use axum::routing::{get, post};
use reqwest::Client;
use runlark_domain::{
    BudgetPolicy, CommandId, CostSummary, CursorSeq, DecisionKind, PolicyDigest, Project,
    ProjectId, Revision, Run, RunId, RunStatus, ServerPromotionAck, SessionId, Task, TaskId,
    TaskKind, TaskStage, TeamCommand, TeamProjectSpec, TeamRunEntry, Timestamp, WorkflowStage,
    WorkspaceId,
};
use runlark_local::{ProjectStore, SessionStore, SyncStore, initialize_schema, open_sqlite};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::sync::atomic::Ordering;
use tempfile::TempDir;

async fn seed_mock_drafts_snapshot(state: &MockTeamServerState) {
    let mut data = state.inner.lock().await;
    let wid = WorkspaceId::from("ws_d_1");
    let pid = ProjectId::from("prj_d");
    data.snapshot.projects.push(TeamProjectSpec {
        project_id: pid.clone(),
        workspace_id: wid,
        name: "Drafts Proj".to_string(),
        default_branch: "main".to_string(),
        max_concurrent_runs: 2,
        budget_policy: BudgetPolicy::default(),
        policy_digest: PolicyDigest::from("pol_d"),
        ownership_epoch: 1,
        revision: Revision(1),
        created_at: Timestamp(1_000),
        updated_at: Timestamp(1_000),
    });
    data.snapshot.tasks = vec![
        Task {
            id: TaskId::from("tsk_inbox"),
            project_id: pid.clone(),
            title: "Inbox Task".to_string(),
            description: "clarify me".to_string(),
            kind: TaskKind::Standard,
            stage: TaskStage::Inbox,
            workflow_stage: WorkflowStage::Clarification,
            plan_text: None,
            plan_version: 1,
            base_ref: Some("main".to_string()),
            head_ref: None,
            pr_url: None,
            active_session_id: None,
            active_run_id: None,
            latest_run_id: None,
            budget_override: None,
            revision: Revision(1),
            created_at: Timestamp(1_000),
            updated_at: Timestamp(1_000),
        },
        Task {
            id: TaskId::from("tsk_plan_run"),
            project_id: pid.clone(),
            title: "Plan With Run".to_string(),
            description: "plan".to_string(),
            kind: TaskKind::Standard,
            stage: TaskStage::Ready,
            workflow_stage: WorkflowStage::Plan,
            plan_text: None,
            plan_version: 1,
            base_ref: Some("main".to_string()),
            head_ref: None,
            pr_url: None,
            active_session_id: Some(SessionId::from("ses_p")),
            active_run_id: Some(RunId::from("run_p")),
            latest_run_id: Some(RunId::from("run_p")),
            budget_override: None,
            revision: Revision(1),
            created_at: Timestamp(1_000),
            updated_at: Timestamp(1_000),
        },
    ];
    data.snapshot.runs.push(TeamRunEntry {
        run: Run {
            id: RunId::from("run_p"),
            session_id: SessionId::from("ses_p"),
            status: RunStatus::Running,
            worktree_path: String::new(),
            revision: Revision(1),
            created_at: Timestamp(1_000),
            updated_at: Timestamp(1_000),
            task_id: Some(TaskId::from("tsk_plan_run")),
            project_id: Some(pid),
            workflow_stage: WorkflowStage::Plan,
            task_kind: TaskKind::Standard,
            workflow_digest: None,
            base_sha: Some("1111".to_string()),
            head_sha: Some("2222".to_string()),
            plan_version: 1,
            policy_digest: Some(PolicyDigest::from("pol_d")),
            budget_policy: BudgetPolicy::default(),
            cost_summary: CostSummary::default(),
            repair_attempts: 0,
            pr_url: None,
        },
        ownership: runlark_domain::RunOwnership::default(),
    });
}

async fn spawn_mock_drafts_server() -> (String, MockTeamServerState, tokio::task::JoinHandle<()>) {
    let state = MockTeamServerState::new("ws_d_1", "Drafts WS", "drafts-ws", None);
    seed_mock_drafts_snapshot(&state).await;
    let router = Router::new()
        .route("/api/v1/auth/whoami", get(mock_whoami))
        .route("/api/v1/workspaces/{wid}/sync/snapshot", get(mock_snapshot))
        .route("/api/v1/workspaces/{wid}/sync/events", get(mock_events))
        .route("/api/v1/workspaces/{wid}/commands", post(mock_commands))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{addr}"), state, handle)
}

fn init_drafts_db(db_path: &Path, repo_dir: &Path) {
    let mut conn = open_sqlite(db_path).unwrap();
    initialize_schema(&conn).unwrap();
    let proj = Project::new(
        ProjectId::from("prj_d"),
        "Drafts Proj",
        repo_dir.to_string_lossy().as_ref(),
        "main",
        BudgetPolicy::default(),
        PolicyDigest::from("pol_d"),
        Timestamp(1_000),
    );
    ProjectStore::create_project(&conn, &proj).unwrap();
    SyncStore::promote_project_to_team_atomic(
        &mut conn,
        &ProjectId::from("prj_d"),
        "http://placeholder",
        &ServerPromotionAck {
            workspace_id: WorkspaceId::from("ws_d_1"),
            project_id: ProjectId::from("prj_d"),
            initial_cursor: CursorSeq(0),
            ownership_epoch: 1,
            acknowledged_at: Timestamp(1_000),
        },
        Timestamp(1_000),
    )
    .unwrap();
}

async fn verify_session_commands_team_gate(
    http: &Client,
    daemon_url: &str,
    mock_state: &MockTeamServerState,
    db_path: &Path,
) {
    mock_state.offline.store(true, Ordering::SeqCst);
    for body in [
        json!({
            "type": "StopRun",
            "payload": {
                "command_id": "cmd_off_stop",
                "session_id": "ses_p",
                "run_id": "run_p",
                "reason": "offline stop",
                "expected_revision": 1,
                "created_at": 100
            }
        }),
        json!({
            "type": "SendPrompt",
            "payload": {
                "command_id": "cmd_off_prompt",
                "session_id": "ses_p",
                "run_id": "run_cmd_offline",
                "prompt": "offline prompt",
                "expected_revision": 1,
                "created_at": 100
            }
        }),
        json!({
            "type": "ResumeSession",
            "payload": {
                "command_id": "cmd_off_resume",
                "session_id": "ses_p",
                "run_id": "run_p",
                "after_seq": null,
                "expected_revision": 1,
                "created_at": 100
            }
        }),
    ] {
        let resp = http
            .post(format!("{daemon_url}/api/v1/commands"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 503);
    }
    assert_offline_chat_prompt_gate(http, daemon_url, db_path).await;

    mock_state.offline.store(false, Ordering::SeqCst);
    let stop_ok = http
        .post(format!("{daemon_url}/api/v1/commands"))
        .json(&json!({
            "type": "StopRun",
            "payload": {
                "command_id": "cmd_route_stop_1",
                "session_id": "ses_p",
                "run_id": "run_p",
                "reason": "operator stop via commands route",
                "expected_revision": 1,
                "created_at": 110
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(stop_ok.status(), 201);

    let data = mock_state.inner.lock().await;
    assert!(data.commands_seen.iter().any(|c| matches!(
        c,
        TeamCommand::CancelRun(r)
            if r.command_id == CommandId::from("cmd_route_stop_1")
                && r.run_id == RunId::from("run_p")
    )));
}

async fn assert_offline_chat_prompt_gate(http: &Client, daemon_url: &str, db_path: &Path) {
    for chat_body in [
        json!({ "input": "offline plain prompt", "session_id": "ses_p", "run_id": "run_chat_offline" }),
        json!({ "input": "offline prompt existing run", "session_id": "ses_p" }),
    ] {
        let resp = http
            .post(format!("{daemon_url}/api/v1/chat/dispatch"))
            .json(&chat_body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 503);
    }

    let conn = open_sqlite(db_path).unwrap();
    assert!(
        SessionStore::get_run(&conn, &RunId::from("run_cmd_offline"))
            .unwrap()
            .is_none()
    );
    assert!(
        SessionStore::get_run(&conn, &RunId::from("run_chat_offline"))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        SessionStore::get_session(&conn, &SessionId::from("ses_p"))
            .unwrap()
            .unwrap()
            .current_revision,
        Revision(1)
    );
    assert_eq!(
        SessionStore::get_run(&conn, &RunId::from("run_p"))
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Running
    );
}

async fn verify_clarify_and_plan_approval(
    http: &Client,
    daemon_url: &str,
    mock_state: &MockTeamServerState,
) {
    mock_state.offline.store(true, Ordering::SeqCst);
    assert_eq!(
        http.post(format!("{daemon_url}/api/v1/tasks/tsk_inbox/clarify"))
            .json(&json!({ "clarification": "cannot mutate offline", "advance_to_plan": true }))
            .send()
            .await
            .unwrap()
            .status(),
        503
    );

    mock_state.offline.store(false, Ordering::SeqCst);
    assert_eq!(
        http.post(format!("{daemon_url}/api/v1/tasks/tsk_inbox/clarify"))
            .json(&json!({ "clarification": "team clarification", "advance_to_plan": true }))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );

    assert_eq!(
        http.post(format!("{daemon_url}/api/v1/tasks/tsk_inbox/plan"))
            .json(&json!({ "plan_text": "no run", "approve": true }))
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    assert_eq!(
        http.post(format!("{daemon_url}/api/v1/tasks/tsk_plan_run/plan"))
            .json(&json!({ "plan_text": "approved plan", "approve": true }))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        http.post(format!("{daemon_url}/api/v1/chat/dispatch"))
            .json(&json!({ "input": "/stop run_p stop from chat" }))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
}

async fn verify_decision_intent_draft(http: &Client, daemon_url: &str) {
    let intent_val: Value = http
        .post(format!("{daemon_url}/api/v1/team/drafts"))
        .json(&json!({
            "draft_id": "draft_dec_intent",
            "project_id": "prj_d",
            "task_id": "tsk_inbox",
            "kind": "decision_intent",
            "payload_json": "{}",
            "queue_sync": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(intent_val["sync_state"], json!({ "state": "local_draft" }));
    assert_eq!(
        http.post(format!(
            "{daemon_url}/api/v1/team/drafts/draft_dec_intent/retry"
        ))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .status(),
        400
    );
}

async fn verify_drafts_edge_cases(
    http: &Client,
    daemon_url: &str,
    mock_state: &MockTeamServerState,
) {
    verify_decision_intent_draft(http, daemon_url).await;

    assert_eq!(
        http.post(format!("{daemon_url}/api/v1/team/drafts"))
            .json(&json!({
                "draft_id": "draft_cmt_det",
                "project_id": "prj_d",
                "task_id": "tsk_inbox",
                "kind": "task_comment",
                "payload_json": json!({ "body": "det body" }).to_string(),
                "queue_sync": true
            }))
            .send()
            .await
            .unwrap()
            .status(),
        201
    );
    let list_before: Value = http
        .get(format!("{daemon_url}/api/v1/team/drafts"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let created_payload: Value = serde_json::from_str(
        list_before["outbox"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["draft_id"] == "draft_cmt_det")
            .unwrap()["payload_json"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        created_payload["payload"]["comment_id"],
        "cmt_draft_cmt_det"
    );

    assert_eq!(
        http.post(format!(
            "{daemon_url}/api/v1/team/drafts/draft_cmt_det/retry"
        ))
        .json(&json!({ "payload_json": "not-json" }))
        .send()
        .await
        .unwrap()
        .status(),
        400
    );
    let after_bad_retry: Value = http
        .get(format!("{daemon_url}/api/v1/team/drafts"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        after_bad_retry["outbox"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["draft_id"] == "draft_cmt_det")
    );

    assert_eq!(
        http.post(format!(
            "{daemon_url}/api/v1/team/drafts/draft_cmt_det/retry"
        ))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .status(),
        200
    );

    let data = mock_state.inner.lock().await;
    assert!(
        data.commands_seen
            .iter()
            .any(|c| matches!(c, TeamCommand::CancelRun(r) if r.run_id == RunId::from("run_p")))
    );
    let cmt_ids: Vec<_> = data
        .commands_seen
        .iter()
        .filter_map(|c| match c {
            TeamCommand::AddComment(a) if a.body == "det body" => Some(a.comment_id.0.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(cmt_ids, vec!["cmt_draft_cmt_det"]);
    assert!(data.commands_seen.iter().any(
        |c| matches!(c, TeamCommand::RecordDecision(d) if d.kind == DecisionKind::ApprovePlan)
    ));
}

#[tokio::test]
async fn clarify_plan_approve_and_draft_determinism_respect_team_gates() {
    let tmp = TempDir::new().unwrap();
    let repo_dir = tmp.path().join("repo");
    fs::create_dir_all(&repo_dir).unwrap();
    let db_path = tmp.path().join("runlark.db");
    init_drafts_db(&db_path, &repo_dir);

    let (mock_url, mock_state, mock_handle) = spawn_mock_drafts_server().await;
    let (daemon_url, _state, _shutdown, daemon_handle) = spawn_daemon(&db_path).await;
    let http = Client::new();

    assert_eq!(
        http.post(format!("{daemon_url}/api/v1/team/login"))
            .json(
                &json!({ "server_url": mock_url, "bearer_token": "tok", "workspace_id": "ws_d_1" })
            )
            .send()
            .await
            .unwrap()
            .status(),
        200
    );

    verify_session_commands_team_gate(&http, &daemon_url, &mock_state, &db_path).await;
    verify_clarify_and_plan_approval(&http, &daemon_url, &mock_state).await;
    verify_drafts_edge_cases(&http, &daemon_url, &mock_state).await;
    verify_team_start_idempotency_and_online_prompt(&http, &daemon_url).await;

    daemon_handle.abort();
    mock_handle.abort();
}

async fn verify_team_start_idempotency_and_online_prompt(http: &Client, daemon_url: &str) {
    let first = http
        .post(format!("{daemon_url}/api/v1/tasks/tsk_inbox/start"))
        .json(&json!({ "command_id": "cmd_start_idemp_1" }))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 201);
    let first_body: Value = first.json().await.unwrap();
    let first_run_id = first_body["run"]["id"].as_str().unwrap().to_string();

    let replay = http
        .post(format!("{daemon_url}/api/v1/tasks/tsk_inbox/start"))
        .json(&json!({ "command_id": "cmd_start_idemp_1" }))
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), 200);
    let replay_body: Value = replay.json().await.unwrap();
    assert_eq!(replay_body["run"]["id"], first_run_id);

    let conflict = http
        .post(format!("{daemon_url}/api/v1/tasks/tsk_inbox/start"))
        .json(&json!({ "command_id": "cmd_start_idemp_1", "runner_id": "rnr_other" }))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), 409);

    let chat_ok = http
        .post(format!("{daemon_url}/api/v1/chat/dispatch"))
        .json(&json!({
            "input": "online prompt via chat",
            "session_id": "ses_p",
            "command_id": "cmd_chat_online_1"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(chat_ok.status(), 201);
}
