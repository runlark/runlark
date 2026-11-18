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
use runlark_cli::TeamSyncWorker;
use runlark_cli::routes::team_decisions::sync_run_evidence_to_server;
use runlark_cli::routes::team_gate::resolve_project_team_context;
use runlark_domain::{
    ArtifactDigest, ArtifactKind, ArtifactRef, BudgetPolicy, CostSummary, CursorSeq, DraftId,
    DraftKind, DraftSyncState, OfflineDraft, PolicyDigest, Project, ProjectId, Question,
    QuestionId, QuestionStatus, Revision, Run, RunId, RunOwnership, RunStatus, ServerPromotionAck,
    Session, SessionId, SessionStatus, Task, TaskId, TaskKind, TaskStage, TeamCommand,
    TeamProjectSpec, TeamRunEntry, Timestamp, UserId, WorkflowStage, WorkspaceId,
};
use runlark_local::{
    ArtifactCasStore, AttemptStore, DraftStore, ProjectStore, QuestionStore, SessionStore,
    SyncOutboxEntry, SyncOutboxStatus, SyncOutboxStore, SyncStore, TeamAuthSession, TeamAuthStore,
    initialize_schema, open_sqlite, sha256_hex,
};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::sync::atomic::Ordering;
use tempfile::TempDir;

async fn mock_gate_commands(
    AxumState(st): AxumState<MockTeamServerState>,
    headers: HeaderMap,
    AxumPath(wid): AxumPath<String>,
    Json(cmd): Json<TeamCommand>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    check_auth(&headers, &st)?;
    if matches!(&cmd, TeamCommand::RecordDecision(d) if d.task_id.0 == "tsk_missing_404") {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "task missing on server" })),
        ));
    }
    mock_commands(AxumState(st), headers, AxumPath(wid), Json(cmd)).await
}

async fn mock_put_artifact(
    AxumState(st): AxumState<MockTeamServerState>,
    headers: HeaderMap,
    AxumPath((_wid, digest)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<ArtifactRef>), (StatusCode, Json<Value>)> {
    check_auth(&headers, &st)?;
    Ok((
        StatusCode::CREATED,
        Json(ArtifactRef {
            digest_sha256: ArtifactDigest::parse(digest).unwrap(),
            kind: ArtifactKind::EvidenceBundle,
            size_bytes: body.len() as u64,
            media_type: "application/json".to_string(),
            created_at: Timestamp(1_000),
        }),
    ))
}

async fn spawn_gate_server() -> (String, MockTeamServerState, tokio::task::JoinHandle<()>) {
    let state = MockTeamServerState::new("ws_gate", "Gate", "gate", Some("tok"));
    let router = Router::new()
        .route("/api/v1/auth/whoami", get(mock_whoami))
        .route("/api/v1/workspaces/{wid}/sync/snapshot", get(mock_snapshot))
        .route("/api/v1/workspaces/{wid}/sync/events", get(mock_events))
        .route(
            "/api/v1/workspaces/{wid}/commands",
            post(mock_gate_commands),
        )
        .route(
            "/api/v1/workspaces/{wid}/artifacts/{digest}",
            put(mock_put_artifact),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{addr}"), state, handle)
}

async fn seed_gate_db(
    db_path: &Path,
    mock_url: &str,
    mock_state: &MockTeamServerState,
    repo_dir: &Path,
) {
    let wid = WorkspaceId::from("ws_gate");
    let pid = ProjectId::from("prj_gate");
    let head = "a".repeat(40);
    let base = "b".repeat(40);

    let mut data = mock_state.inner.lock().await;
    data.snapshot.projects.push(TeamProjectSpec {
        project_id: pid.clone(),
        workspace_id: wid.clone(),
        name: "Gate".to_string(),
        default_branch: "main".to_string(),
        max_concurrent_runs: 2,
        budget_policy: BudgetPolicy::default(),
        policy_digest: PolicyDigest::from("pol_project"),
        ownership_epoch: 1,
        revision: Revision(1),
        created_at: Timestamp(1_000),
        updated_at: Timestamp(1_000),
    });
    data.snapshot.tasks.push(Task {
        id: TaskId::from("tsk_acc"),
        project_id: pid.clone(),
        title: "Accept".to_string(),
        description: "desc".to_string(),
        kind: TaskKind::Standard,
        stage: TaskStage::Acceptance,
        workflow_stage: WorkflowStage::HumanAcceptance,
        plan_text: Some("plan".to_string()),
        plan_version: 1,
        base_ref: Some("main".to_string()),
        head_ref: None,
        pr_url: None,
        active_session_id: Some(SessionId::from("ses_acc")),
        active_run_id: Some(RunId::from("run_acc")),
        latest_run_id: Some(RunId::from("run_acc")),
        budget_override: None,
        revision: Revision(2),
        created_at: Timestamp(1_000),
        updated_at: Timestamp(1_000),
    });
    data.snapshot.runs.push(TeamRunEntry {
        run: Run {
            id: RunId::from("run_acc"),
            session_id: SessionId::from("ses_acc"),
            status: RunStatus::WaitingForApproval,
            worktree_path: repo_dir.to_string_lossy().to_string(),
            revision: Revision(1),
            created_at: Timestamp(1_000),
            updated_at: Timestamp(1_000),
            task_id: Some(TaskId::from("tsk_acc")),
            project_id: Some(pid.clone()),
            workflow_stage: WorkflowStage::HumanAcceptance,
            task_kind: TaskKind::Standard,
            workflow_digest: None,
            base_sha: Some(base),
            head_sha: Some(head),
            plan_version: 1,
            policy_digest: Some(PolicyDigest::from("pol_pinned_run")),
            budget_policy: BudgetPolicy::default(),
            cost_summary: CostSummary::default(),
            repair_attempts: 0,
            pr_url: None,
        },
        ownership: RunOwnership::default(),
    });
    let snap = data.snapshot.clone();
    drop(data);
    seed_local_gate_db(db_path, mock_url, repo_dir, &snap);
}

fn seed_local_gate_db(
    db_path: &Path,
    mock_url: &str,
    repo_dir: &Path,
    snap: &runlark_domain::WorkspaceSnapshot,
) {
    let wid = WorkspaceId::from("ws_gate");
    let pid = ProjectId::from("prj_gate");
    let mut conn = open_sqlite(db_path).unwrap();
    initialize_schema(&conn).unwrap();
    ProjectStore::create_project(
        &conn,
        &Project::new(
            pid.clone(),
            "Gate",
            repo_dir.to_string_lossy().as_ref(),
            "main",
            BudgetPolicy::default(),
            PolicyDigest::from("pol_project"),
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
            runner_id: None,
            runner_name: None,
            updated_at: Timestamp(1_000),
        },
    )
    .unwrap();
    SyncStore::apply_snapshot_atomic(&mut conn, mock_url, snap, Timestamp(1_100)).unwrap();

    let cas = ArtifactCasStore::from_data_dir(db_path.parent().unwrap());
    let art = cas
        .put_bytes(
            &conn,
            ArtifactKind::EvidenceBundle,
            "application/json",
            b"{\"verdict\":\"pass\"}",
            Timestamp(1_200),
        )
        .unwrap();
    ArtifactCasStore::link_to_run(
        &conn,
        &RunId::from("run_acc"),
        &art.digest_sha256,
        "evidence_bundle",
        Timestamp(1_200),
    )
    .unwrap();
    SessionStore::update_session_with_revision(
        &conn,
        &Session {
            id: SessionId::from("ses_acc"),
            task_id: TaskId::from("tsk_acc"),
            cwd: repo_dir.to_string_lossy().to_string(),
            status: SessionStatus::Active,
            current_revision: Revision(1),
            active_run_id: Some(RunId::from("run_acc")),
            created_at: Timestamp(1_000),
            updated_at: Timestamp(1_000),
        },
        Revision(1),
    )
    .unwrap();
    QuestionStore::create_question(
        &conn,
        &Question {
            id: QuestionId::from("q_gate_1"),
            session_id: SessionId::from("ses_acc"),
            run_id: RunId::from("run_acc"),
            prompt: "Approve?".to_string(),
            options: vec!["yes".to_string()],
            status: QuestionStatus::Pending,
            answer: None,
            created_at: Timestamp(1_200),
            updated_at: Timestamp(1_200),
        },
    )
    .unwrap();
}

fn enqueue_404_and_followup(db_path: &Path) {
    let conn = open_sqlite(db_path).unwrap();
    DraftStore::save_draft(
        &conn,
        &OfflineDraft {
            id: DraftId::from("draft_404"),
            project_id: ProjectId::from("prj_gate"),
            task_id: None,
            kind: DraftKind::PlanEdit,
            payload_json: "{}".to_string(),
            base_revision: Some(Revision(1)),
            sync_state: DraftSyncState::PendingSync,
            created_at: Timestamp(5_000),
            updated_at: Timestamp(5_000),
        },
    )
    .unwrap();
    let bad_cmd = TeamCommand::RecordDecision(runlark_domain::RecordDecisionCommand {
        command_id: runlark_domain::CommandId::from("cmd_404_dec"),
        decision_id: runlark_domain::DecisionId::from("dec_404"),
        task_id: TaskId::from("tsk_missing_404"),
        run_id: RunId::from("run_acc"),
        kind: runlark_domain::DecisionKind::Accept,
        actor: "Owner".to_string(),
        plan_version: 1,
        head_sha: "a".repeat(40),
        base_sha: "b".repeat(40),
        policy_digest: PolicyDigest::from("pol_pinned_run"),
        evidence_digest: ArtifactDigest::parse(sha256_hex(b"{\"verdict\":\"pass\"}")).unwrap(),
        comment: None,
        expected_revision: Revision(1),
        created_at: Timestamp(5_000),
    });
    SyncOutboxStore::enqueue(
        &conn,
        &SyncOutboxEntry {
            command_id: runlark_domain::CommandId::from("cmd_404_dec"),
            workspace_id: WorkspaceId::from("ws_gate"),
            project_id: Some(ProjectId::from("prj_gate")),
            draft_id: Some(DraftId::from("draft_404")),
            command_type: "record_decision".to_string(),
            payload_json: serde_json::to_string(&bad_cmd).unwrap(),
            expected_revision: Some(Revision(1)),
            status: SyncOutboxStatus::Pending,
            last_error: None,
            conflict_server_revision: None,
            created_at: Timestamp(5_000),
            updated_at: Timestamp(5_000),
        },
    )
    .unwrap();
    let ok_cmt = TeamCommand::AddComment(runlark_domain::AddCommentCommand {
        command_id: runlark_domain::CommandId::from("cmd_after_404"),
        comment_id: runlark_domain::CommentId::from("cmt_after_404"),
        task_id: TaskId::from("tsk_acc"),
        run_id: None,
        body: "after 404".to_string(),
        created_at: Timestamp(5_001),
    });
    SyncOutboxStore::enqueue(
        &conn,
        &SyncOutboxEntry {
            command_id: runlark_domain::CommandId::from("cmd_after_404"),
            workspace_id: WorkspaceId::from("ws_gate"),
            project_id: Some(ProjectId::from("prj_gate")),
            draft_id: None,
            command_type: "add_comment".to_string(),
            payload_json: serde_json::to_string(&ok_cmt).unwrap(),
            expected_revision: None,
            status: SyncOutboxStatus::Pending,
            last_error: None,
            conflict_server_revision: None,
            created_at: Timestamp(5_001),
            updated_at: Timestamp(5_001),
        },
    )
    .unwrap();
}

async fn assert_offline_gates_and_online_accept(
    http: &Client,
    daemon_url: &str,
    mock_state: &MockTeamServerState,
    state: &runlark_cli::AppState,
    db_path: &Path,
) {
    mock_state.offline.store(true, Ordering::SeqCst);
    for (path, body) in [
        (
            "/api/v1/questions/q_gate_1/answer",
            json!({ "answer": "yes" }),
        ),
        (
            "/api/v1/chat/dispatch",
            json!({ "input": "/answer q_gate_1 yes" }),
        ),
        (
            "/api/v1/runs/run_acc/attempts",
            json!({ "workflow_stage": "implementation", "outcome": { "status": "succeeded" } }),
        ),
    ] {
        assert_eq!(
            http.post(format!("{daemon_url}{path}"))
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            503
        );
    }
    {
        let conn = open_sqlite(db_path).unwrap();
        assert_eq!(
            QuestionStore::get_question(&conn, &QuestionId::from("q_gate_1"))
                .unwrap()
                .unwrap()
                .status,
            QuestionStatus::Pending
        );
        assert!(
            AttemptStore::list_attempts_for_run(&conn, &RunId::from("run_acc"))
                .unwrap()
                .is_empty()
        );
    }

    mock_state.offline.store(false, Ordering::SeqCst);
    assert_eq!(
        http.post(format!("{daemon_url}/api/v1/runs/run_acc/attempts"))
            .json(&json!({
                "command_id": "cmd_att_team_1",
                "workflow_stage": "implementation",
                "outcome": { "status": "succeeded" },
                "cost_micro_usd": 50
            }))
            .send()
            .await
            .unwrap()
            .status(),
        201
    );

    let team_ctx = resolve_project_team_context(state, &ProjectId::from("prj_gate"))
        .unwrap()
        .unwrap();
    sync_run_evidence_to_server(state, &team_ctx, &RunId::from("run_acc"), Timestamp(4_000))
        .await
        .unwrap();
    assert_eq!(
        http.post(format!("{daemon_url}/api/v1/tasks/tsk_acc/accept"))
            .json(&json!({
                "head_sha": "a".repeat(40),
                "base_sha": "b".repeat(40),
                "expected_revision": 2
            }))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
}

#[tokio::test]
async fn question_attempt_accept_policy_idempotency_and_404_outbox_work() {
    let tmp = TempDir::new().unwrap();
    let repo_dir = tmp.path().join("repo");
    fs::create_dir_all(&repo_dir).unwrap();
    let db_path = tmp.path().join("runlark.db");
    let (mock_url, mock_state, mock_handle) = spawn_gate_server().await;
    seed_gate_db(&db_path, &mock_url, &mock_state, &repo_dir).await;

    let (daemon_url, state, _sd, daemon_handle) = spawn_daemon(&db_path).await;
    let http = Client::new();
    assert_offline_gates_and_online_accept(&http, &daemon_url, &mock_state, &state, &db_path).await;

    enqueue_404_and_followup(&db_path);
    let sync_res = TeamSyncWorker::new(state)
        .sync_workspace(&mock_url, "tok", &WorkspaceId::from("ws_gate"))
        .await
        .unwrap();
    assert_eq!(sync_res.outbox_acked, 1);
    assert_eq!(sync_res.outbox_conflicts, 1);

    {
        let conn = open_sqlite(&db_path).unwrap();
        assert_eq!(
            SyncOutboxStore::get(&conn, &runlark_domain::CommandId::from("cmd_404_dec"))
                .unwrap()
                .unwrap()
                .status,
            SyncOutboxStatus::Failed
        );
        assert!(matches!(
            DraftStore::get_draft(&conn, &DraftId::from("draft_404"))
                .unwrap()
                .unwrap()
                .sync_state,
            DraftSyncState::Conflict { .. }
        ));
    }

    let data = mock_state.inner.lock().await;
    assert!(
        data.commands_seen.iter().any(
            |c| matches!(c, TeamCommand::RecordAttempt(a) if a.command_id.0 == "cmd_att_team_1")
        )
    );
    let attach_ids: Vec<_> = data
        .commands_seen
        .iter()
        .filter_map(|c| match c {
            TeamCommand::AttachArtifact(a) => Some(a.command_id.0.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(attach_ids.len(), 2);
    assert_eq!(attach_ids[0], attach_ids[1]);
    assert!(data.commands_seen.iter().any(|c| matches!(
        c,
        TeamCommand::RecordDecision(d)
            if d.kind == runlark_domain::DecisionKind::Accept
                && d.policy_digest == PolicyDigest::from("pol_pinned_run")
    )));
    drop(data);

    daemon_handle.abort();
    mock_handle.abort();
}
