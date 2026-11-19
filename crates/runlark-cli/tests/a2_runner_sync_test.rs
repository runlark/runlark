mod a2_team_mock;

use a2_team_mock::{
    MockTeamServerState, check_auth, mock_commands, mock_events, mock_snapshot, mock_whoami,
    spawn_daemon,
};
use axum::extract::{Extension, Path as AxumPath, State as AxumState};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use runlark_cli::cli::{Cli, run_cli};
use runlark_cli::{RunOwnershipResponse, RunnerAgentWorker, RunnerStateInput, TeamServerError};
use runlark_domain::{
    BudgetPolicy, CostSummary, CursorSeq, LeaseEpoch, PolicyDigest, Project, ProjectId, Revision,
    Run, RunId, RunOwnership, RunStatus, RunnerId, RunnerRegistration, RunnerRunReport,
    RunnerStatus, ServerPromotionAck, SessionId, Task, TaskId, TaskKind, TaskStage, TeamCommand,
    TeamProjectSpec, TeamRunEntry, Timestamp, UserId, WorkflowStage, WorkspaceEventEnvelope,
    WorkspaceId, WorkspaceSyncEvent,
};
use runlark_local::{ProjectStore, RunnerBufferStore, SyncStore, initialize_schema, open_sqlite};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tempfile::TempDir;
use tokio::sync::Mutex;

type SeqMap = Arc<Mutex<HashMap<String, Vec<u64>>>>;

async fn mock_register_runner(
    AxumState(st): AxumState<MockTeamServerState>,
    headers: HeaderMap,
    AxumPath(wid): AxumPath<String>,
    Json(body): Json<Value>,
) -> Result<Json<RunnerRegistration>, (StatusCode, Json<Value>)> {
    check_auth(&headers, &st)?;
    let mut data = st.inner.lock().await;
    let rid = body["runner_id"]
        .as_str()
        .unwrap_or("rnr_local")
        .to_string();
    let name = body["name"].as_str().unwrap_or("Local Runner").to_string();
    let reg = RunnerRegistration {
        id: RunnerId::from(rid.clone()),
        workspace_id: WorkspaceId::from(wid),
        owner_user_id: UserId::from("usr_owner"),
        name,
        status: RunnerStatus::Online,
        last_heartbeat_at: Timestamp(1_000),
        created_at: Timestamp(1_000),
    };
    data.snapshot.runners.retain(|r| r.id.as_ref() != rid);
    data.snapshot.runners.push(reg.clone());
    Ok(Json(reg))
}

async fn mock_heartbeat(
    AxumState(st): AxumState<MockTeamServerState>,
    headers: HeaderMap,
    AxumPath((_wid, rid)): AxumPath<(String, String)>,
) -> Result<Json<RunnerRegistration>, (StatusCode, Json<Value>)> {
    check_auth(&headers, &st)?;
    let mut data = st.inner.lock().await;
    let reg = data
        .snapshot
        .runners
        .iter_mut()
        .find(|r| r.id.as_ref() == rid)
        .unwrap();
    reg.last_heartbeat_at = Timestamp(2_000);
    Ok(Json(reg.clone()))
}

async fn mock_assign(
    AxumState(st): AxumState<MockTeamServerState>,
    headers: HeaderMap,
    AxumPath((wid, run_id)): AxumPath<(String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<RunOwnershipResponse>, (StatusCode, Json<Value>)> {
    check_auth(&headers, &st)?;
    let mut data = st.inner.lock().await;
    let runner_id = body["runner_id"].as_str().unwrap().to_string();
    let updated = {
        let entry = data
            .snapshot
            .runs
            .iter_mut()
            .find(|e| e.run.id.as_ref() == run_id)
            .unwrap();
        if let Some(owner) = &entry.ownership.assigned_runner_id
            && owner.as_ref() != runner_id
        {
            return Err((
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "active_run_prevents_takeover",
                    "message": format!("already owned by {owner}"),
                    "current_revision": entry.run.revision.0
                })),
            ));
        }
        entry.ownership = RunOwnership {
            assigned_runner_id: Some(RunnerId::from(runner_id)),
            lease_epoch: LeaseEpoch(1),
            runner_unreachable: false,
        };
        entry.run.revision = Revision(entry.run.revision.0 + 1);
        (entry.run.clone(), entry.ownership.clone())
    };
    data.cursor += 1;
    let seq = CursorSeq(data.cursor);
    data.snapshot.cursor_seq = seq;
    data.events.push(WorkspaceEventEnvelope {
        cursor_seq: seq,
        workspace_id: WorkspaceId::from(wid),
        project_id: updated.0.project_id.clone(),
        aggregate_type: "run".to_string(),
        aggregate_id: run_id,
        event: WorkspaceSyncEvent::RunUpserted {
            run: updated.0.clone(),
            ownership: updated.1.clone(),
        },
        actor_user_id: UserId::from("usr_owner"),
        created_at: Timestamp(3_000),
    });
    Ok(Json(RunOwnershipResponse {
        run: updated.0,
        ownership: updated.1,
    }))
}

async fn mock_runner_state(
    AxumState(st): AxumState<MockTeamServerState>,
    Extension(seqs): Extension<SeqMap>,
    headers: HeaderMap,
    AxumPath((_wid, run_id)): AxumPath<(String, String)>,
    Json(report): Json<RunnerRunReport>,
) -> Result<Json<RunOwnershipResponse>, (StatusCode, Json<Value>)> {
    check_auth(&headers, &st)?;
    let out = {
        let mut data = st.inner.lock().await;
        let entry = data
            .snapshot
            .runs
            .iter_mut()
            .find(|e| e.run.id.as_ref() == run_id)
            .unwrap();
        if entry.ownership.assigned_runner_id.as_ref() != Some(&report.runner_id)
            || entry.ownership.lease_epoch != report.lease_epoch
        {
            return Err((
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "runner_fencing_mismatch",
                    "message": "fencing mismatch",
                    "current_revision": entry.run.revision.0
                })),
            ));
        }
        if let Some(status) = report.status {
            entry.run.status = status;
            entry.run.revision = Revision(entry.run.revision.0 + 1);
        }
        RunOwnershipResponse {
            run: entry.run.clone(),
            ownership: entry.ownership.clone(),
        }
    };
    let report_seqs: Vec<u64> = report.events.iter().map(|e| e.seq.0).collect();
    seqs.lock()
        .await
        .entry(run_id)
        .or_default()
        .extend(report_seqs);
    Ok(Json(out))
}

fn make_mock_run(rid: &str) -> Run {
    Run {
        id: RunId::from(rid),
        session_id: SessionId::from(format!("ses_{rid}")),
        status: RunStatus::Running,
        worktree_path: String::new(),
        revision: Revision(1),
        created_at: Timestamp(1_000),
        updated_at: Timestamp(1_000),
        task_id: Some(TaskId::from("tsk_r")),
        project_id: Some(ProjectId::from("prj_r")),
        workflow_stage: WorkflowStage::Implementation,
        task_kind: TaskKind::Standard,
        workflow_digest: None,
        base_sha: Some("1111".to_string()),
        head_sha: None,
        plan_version: 1,
        policy_digest: Some(PolicyDigest::from("pol_r")),
        budget_policy: BudgetPolicy::default(),
        cost_summary: CostSummary::default(),
        repair_attempts: 0,
        pr_url: None,
    }
}

async fn seed_mock_runner_snapshot(state: &MockTeamServerState) {
    let wid = WorkspaceId::from("ws_rnr_1");
    let mut data = state.inner.lock().await;
    data.snapshot.projects.push(TeamProjectSpec {
        project_id: ProjectId::from("prj_r"),
        workspace_id: wid.clone(),
        name: "Runner Proj".to_string(),
        default_branch: "main".to_string(),
        max_concurrent_runs: 2,
        budget_policy: BudgetPolicy::default(),
        policy_digest: PolicyDigest::from("pol_r"),
        ownership_epoch: 1,
        revision: Revision(1),
        created_at: Timestamp(1_000),
        updated_at: Timestamp(1_000),
    });
    data.snapshot.tasks.push(Task {
        id: TaskId::from("tsk_r"),
        project_id: ProjectId::from("prj_r"),
        title: "Runner Task".to_string(),
        description: "exec".to_string(),
        kind: TaskKind::Standard,
        stage: TaskStage::InProgress,
        workflow_stage: WorkflowStage::Implementation,
        plan_text: None,
        plan_version: 1,
        base_ref: Some("main".to_string()),
        head_ref: None,
        pr_url: None,
        active_session_id: None,
        active_run_id: Some(RunId::from("run_mine")),
        latest_run_id: Some(RunId::from("run_mine")),
        budget_override: None,
        revision: Revision(1),
        created_at: Timestamp(1_000),
        updated_at: Timestamp(1_000),
    });
    data.snapshot.runners.push(RunnerRegistration {
        id: RunnerId::from("rnr_other"),
        workspace_id: wid,
        owner_user_id: UserId::from("usr_owner"),
        name: "Other Runner".to_string(),
        status: RunnerStatus::Unreachable,
        last_heartbeat_at: Timestamp(500),
        created_at: Timestamp(500),
    });
    data.snapshot.runs = vec![
        TeamRunEntry {
            run: make_mock_run("run_mine"),
            ownership: RunOwnership::default(),
        },
        TeamRunEntry {
            run: make_mock_run("run_other"),
            ownership: RunOwnership {
                assigned_runner_id: Some(RunnerId::from("rnr_other")),
                lease_epoch: LeaseEpoch(1),
                runner_unreachable: true,
            },
        },
    ];
}

async fn spawn_mock_runner_server() -> (
    String,
    MockTeamServerState,
    SeqMap,
    tokio::task::JoinHandle<()>,
) {
    let state = MockTeamServerState::new("ws_rnr_1", "Runner WS", "runner-ws", None);
    let seqs: SeqMap = Arc::new(Mutex::new(HashMap::new()));
    seed_mock_runner_snapshot(&state).await;
    let router = Router::new()
        .route("/api/v1/auth/whoami", get(mock_whoami))
        .route("/api/v1/workspaces/{wid}/sync/snapshot", get(mock_snapshot))
        .route("/api/v1/workspaces/{wid}/sync/events", get(mock_events))
        .route(
            "/api/v1/workspaces/{wid}/runners/register",
            post(mock_register_runner),
        )
        .route(
            "/api/v1/workspaces/{wid}/runners/{rid}/heartbeat",
            post(mock_heartbeat),
        )
        .route(
            "/api/v1/workspaces/{wid}/runs/{run_id}/assign",
            post(mock_assign),
        )
        .route(
            "/api/v1/workspaces/{wid}/runs/{run_id}/runner-state",
            post(mock_runner_state),
        )
        .route("/api/v1/workspaces/{wid}/commands", post(mock_commands))
        .layer(Extension(seqs.clone()))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{addr}"), state, seqs, handle)
}

fn init_runner_test_db(db_path: &std::path::Path, repo_dir: &std::path::Path) {
    let mut conn = open_sqlite(db_path).unwrap();
    initialize_schema(&conn).unwrap();
    let proj = Project::new(
        ProjectId::from("prj_r"),
        "Runner Proj",
        repo_dir.to_string_lossy().as_ref(),
        "main",
        BudgetPolicy::default(),
        PolicyDigest::from("pol_r"),
        Timestamp(1_000),
    );
    ProjectStore::create_project(&conn, &proj).unwrap();
    SyncStore::promote_project_to_team_atomic(
        &mut conn,
        &ProjectId::from("prj_r"),
        "http://placeholder",
        &ServerPromotionAck {
            workspace_id: WorkspaceId::from("ws_rnr_1"),
            project_id: ProjectId::from("prj_r"),
            initial_cursor: CursorSeq(0),
            ownership_epoch: 1,
            acknowledged_at: Timestamp(1_000),
        },
        Timestamp(1_000),
    )
    .unwrap();
}

async fn run_cli_runner_setup(daemon_url: &str, mock_url: &str) {
    for argv in [
        vec![
            "runlark",
            "--daemon-url",
            daemon_url,
            "--json",
            "team",
            "login",
            "--server",
            mock_url,
            "--token",
            "tok",
            "--workspace",
            "ws_rnr_1",
        ],
        vec![
            "runlark",
            "--daemon-url",
            daemon_url,
            "--json",
            "team",
            "sync",
        ],
        vec![
            "runlark",
            "--daemon-url",
            daemon_url,
            "--json",
            "runner",
            "register",
            "--name",
            "Local Runner",
            "--runner-id",
            "rnr_local",
        ],
        vec![
            "runlark",
            "--daemon-url",
            daemon_url,
            "--json",
            "runner",
            "assign",
            "run_mine",
            "--runner-id",
            "rnr_local",
        ],
        vec![
            "runlark",
            "--daemon-url",
            daemon_url,
            "--json",
            "runner",
            "status",
        ],
    ] {
        run_cli(Cli::try_parse_from(argv).unwrap()).await.unwrap();
    }
}

#[tokio::test]
async fn runner_fencing_offline_buffering_ordered_flush_and_cli_commands_work() {
    let tmp = TempDir::new().unwrap();
    let repo_dir = tmp.path().join("repo");
    fs::create_dir_all(&repo_dir).unwrap();
    let db_path = tmp.path().join("runlark.db");
    init_runner_test_db(&db_path, &repo_dir);

    let (mock_url, mock_state, runner_seqs, mock_handle) = spawn_mock_runner_server().await;
    let (daemon_url, state, _shutdown, daemon_handle) = spawn_daemon(&db_path).await;
    run_cli_runner_setup(&daemon_url, &mock_url).await;

    let agent = RunnerAgentWorker::new(state);
    let wid = WorkspaceId::from("ws_rnr_1");
    let assigned = agent
        .list_assigned_executable_runs(&wid, &RunnerId::from("rnr_local"))
        .unwrap();
    assert_eq!(assigned.len(), 1);
    assert_eq!(assigned[0].0.id, RunId::from("run_mine"));

    let stale_err = agent
        .report_or_buffer_run_state(RunnerStateInput {
            workspace_id: &wid,
            run_id: &RunId::from("run_mine"),
            runner_id: &RunnerId::from("rnr_local"),
            lease_epoch: LeaseEpoch(99),
            status: None,
            workflow_stage: None,
            head_sha: None,
            base_sha: None,
            event_type: "step",
            payload_json: "{}",
        })
        .await
        .unwrap_err();
    assert!(matches!(stale_err, TeamServerError::Conflict { .. }));

    mock_state.offline.store(true, Ordering::SeqCst);
    for seq_no in [1, 2] {
        let sent_now = agent
            .report_or_buffer_run_state(RunnerStateInput {
                workspace_id: &wid,
                run_id: &RunId::from("run_mine"),
                runner_id: &RunnerId::from("rnr_local"),
                lease_epoch: LeaseEpoch(1),
                status: None,
                workflow_stage: Some(WorkflowStage::Verification),
                head_sha: Some("deadbeef".to_string()),
                base_sha: Some("1111".to_string()),
                event_type: "runner.step",
                payload_json: &format!("{{\"step\":{seq_no}}}"),
            })
            .await
            .unwrap();
        assert!(!sent_now);
    }
    assert_eq!(
        RunnerBufferStore::list_unflushed(&open_sqlite(&db_path).unwrap(), Some(&wid))
            .unwrap()
            .len(),
        2
    );

    mock_state.offline.store(false, Ordering::SeqCst);
    let summary = agent.tick_once().await.unwrap();
    assert_eq!(summary.buffered_events_flushed, 2);
    assert_eq!(summary.heartbeats_sent, 1);
    assert!(
        RunnerBufferStore::list_unflushed(&open_sqlite(&db_path).unwrap(), Some(&wid))
            .unwrap()
            .is_empty()
    );
    assert_eq!(runner_seqs.lock().await.get("run_mine"), Some(&vec![1, 2]));

    verify_mixed_epoch_and_cross_run_isolation(&agent, &mock_state, &db_path, &runner_seqs).await;

    run_cli(
        Cli::try_parse_from([
            "runlark",
            "--daemon-url",
            &daemon_url,
            "--json",
            "runner",
            "cancel-unreachable",
            "run_other",
            "--reason",
            "laptop powered off",
        ])
        .unwrap(),
    )
    .await
    .unwrap();
    assert!(mock_state.inner.lock().await.commands_seen.iter().any(|c| {
        matches!(c, TeamCommand::ForceCancelUnreachableRun(f) if f.run_id == RunId::from("run_other"))
    }));

    daemon_handle.abort();
    mock_handle.abort();
}

fn buffer_step_event(
    db_path: &std::path::Path,
    run_id: &RunId,
    seq: u64,
    epoch: u64,
    workflow_stage: Option<WorkflowStage>,
) {
    let conn = open_sqlite(db_path).unwrap();
    RunnerBufferStore::buffer_report(
        &conn,
        &WorkspaceId::from("ws_rnr_1"),
        run_id,
        &RunnerRunReport {
            runner_id: RunnerId::from("rnr_local"),
            lease_epoch: LeaseEpoch(epoch),
            status: None,
            workflow_stage,
            head_sha: None,
            base_sha: None,
            events: vec![runlark_domain::RunnerEventInput {
                seq: runlark_domain::EventSeq(seq),
                event_type: "runner.step".to_string(),
                payload_json: "{}".to_string(),
                created_at: Timestamp(5_000 + seq),
            }],
        },
    )
    .unwrap();
}

async fn verify_mixed_epoch_and_cross_run_isolation(
    agent: &RunnerAgentWorker,
    mock_state: &MockTeamServerState,
    db_path: &std::path::Path,
    runner_seqs: &SeqMap,
) {
    let wid = WorkspaceId::from("ws_rnr_1");
    let mine = RunId::from("run_mine");
    let other = RunId::from("run_other");

    // 1. Conflicting buffered event on run_other must not break reporting on run_mine.
    buffer_step_event(db_path, &other, 1, 1, None);
    let ok_mine = agent
        .report_or_buffer_run_state(RunnerStateInput {
            workspace_id: &wid,
            run_id: &mine,
            runner_id: &RunnerId::from("rnr_local"),
            lease_epoch: LeaseEpoch(1),
            status: None,
            workflow_stage: None,
            head_sha: None,
            base_sha: None,
            event_type: "runner.step",
            payload_json: "{\"step\":3}",
        })
        .await
        .unwrap();
    assert!(ok_mine);

    // 2. Mixed-epoch flush: seq 4 (epoch 2 ok), seq 5 & 6 (two chunks with stale epoch 1 -> 2 conflicts), seq 7 (epoch 2 ok).
    {
        let mut data = mock_state.inner.lock().await;
        let entry = data
            .snapshot
            .runs
            .iter_mut()
            .find(|e| e.run.id == mine)
            .unwrap();
        entry.ownership.lease_epoch = LeaseEpoch(2);
    }
    for (seq_no, epoch, stage) in [
        (4, 2, None),
        (5, 1, None),
        (6, 1, Some(WorkflowStage::Verification)),
        (7, 2, None),
    ] {
        buffer_step_event(db_path, &mine, seq_no, epoch, stage);
    }
    let summary = agent.tick_once().await.unwrap();
    assert_eq!(summary.buffered_events_flushed, 2);
    assert_eq!(summary.fencing_conflicts, 3);
    assert!(
        RunnerBufferStore::list_unflushed(&open_sqlite(db_path).unwrap(), Some(&wid))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        runner_seqs.lock().await.get("run_mine"),
        Some(&vec![1, 2, 3, 4, 7])
    );
}
