use axum::Json;
use axum::extract::{Path as AxumPath, Query, State as AxumState};
use axum::http::{HeaderMap, StatusCode};
use runlark_cli::{AppState, build_router};
use runlark_domain::{
    BudgetPolicy, CommentId, CostSummary, CursorSeq, PolicyDigest, Revision, Run, RunOwnership,
    RunStatus, SessionId, TaskComment, TaskId, TaskStage, TeamCommand, TeamRunEntry, Timestamp,
    UserId, WorkflowStage, Workspace, WorkspaceEventEnvelope, WorkspaceId, WorkspaceMember,
    WorkspaceRole, WorkspaceSnapshot, WorkspaceSyncEvent,
};
use runlark_local::{initialize_schema, open_sqlite, sha256_hex};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, watch};

#[derive(Clone)]
pub struct MockTeamServerState {
    pub offline: Arc<AtomicBool>,
    pub expected_token: Option<String>,
    pub inner: Arc<Mutex<MockTeamData>>,
}

pub struct MockTeamData {
    pub cursor: u64,
    pub snapshot: WorkspaceSnapshot,
    pub events: Vec<WorkspaceEventEnvelope>,
    pub commands_seen: Vec<TeamCommand>,
    pub commands_by_id: HashMap<String, (String, Value)>,
    pub current_task_rev: u64,
    pub emit_gap_once: bool,
}

#[derive(Deserialize)]
pub struct EventsQuery {
    pub after_cursor: Option<u64>,
    pub limit: Option<usize>,
}

impl MockTeamServerState {
    pub fn new(workspace_id: &str, name: &str, slug: &str, expected_token: Option<&str>) -> Self {
        let wid = WorkspaceId::from(workspace_id);
        let ws = Workspace {
            id: wid.clone(),
            name: name.to_string(),
            slug: slug.to_string(),
            created_by: UserId::from("usr_owner"),
            revision: Revision(1),
            created_at: Timestamp(1_000),
            updated_at: Timestamp(1_000),
        };
        let member = WorkspaceMember {
            workspace_id: wid,
            user_id: UserId::from("usr_owner"),
            display_name: "Owner".to_string(),
            role: WorkspaceRole::Owner,
            joined_at: Timestamp(1_000),
        };
        Self {
            offline: Arc::new(AtomicBool::new(false)),
            expected_token: expected_token.map(str::to_string),
            inner: Arc::new(Mutex::new(MockTeamData {
                cursor: 1,
                snapshot: WorkspaceSnapshot {
                    workspace: ws,
                    cursor_seq: CursorSeq(1),
                    members: vec![member],
                    projects: Vec::new(),
                    tasks: Vec::new(),
                    runs: Vec::new(),
                    attempts: Vec::new(),
                    decisions: Vec::new(),
                    comments: Vec::new(),
                    artifacts: Vec::new(),
                    runners: Vec::new(),
                },
                events: Vec::new(),
                commands_seen: Vec::new(),
                commands_by_id: HashMap::new(),
                current_task_rev: 1,
                emit_gap_once: false,
            })),
        }
    }
}

pub fn check_auth(
    headers: &HeaderMap,
    st: &MockTeamServerState,
) -> Result<(), (StatusCode, Json<Value>)> {
    if st.offline.load(Ordering::SeqCst) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "offline", "message": "simulated server offline" })),
        ));
    }
    if let Some(expected) = &st.expected_token {
        let auth = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        if auth != format!("Bearer {expected}") {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "unauthorized", "message": "bad token" })),
            ));
        }
    }
    Ok(())
}

pub async fn mock_whoami(
    AxumState(st): AxumState<MockTeamServerState>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    check_auth(&headers, &st)?;
    Ok(Json(
        json!({ "user_id": "usr_owner", "display_name": "Owner" }),
    ))
}

pub async fn mock_snapshot(
    AxumState(st): AxumState<MockTeamServerState>,
    headers: HeaderMap,
) -> Result<Json<WorkspaceSnapshot>, (StatusCode, Json<Value>)> {
    check_auth(&headers, &st)?;
    let data = st.inner.lock().await;
    Ok(Json(data.snapshot.clone()))
}

pub async fn mock_events(
    AxumState(st): AxumState<MockTeamServerState>,
    headers: HeaderMap,
    Query(q): Query<EventsQuery>,
) -> Result<Json<Vec<WorkspaceEventEnvelope>>, (StatusCode, Json<Value>)> {
    check_auth(&headers, &st)?;
    let mut data = st.inner.lock().await;
    if data.emit_gap_once {
        data.emit_gap_once = false;
        let jumped = data.cursor + 10;
        let pid = data.snapshot.projects[0].project_id.clone();
        return Ok(Json(vec![WorkspaceEventEnvelope {
            workspace_id: data.snapshot.workspace.id.clone(),
            project_id: Some(pid),
            aggregate_type: "task".to_string(),
            aggregate_id: "tsk_ready".to_string(),
            actor_user_id: UserId::from("usr_owner"),
            cursor_seq: CursorSeq(jumped),
            event: WorkspaceSyncEvent::CommentAppended {
                comment: TaskComment {
                    id: CommentId::from("cmt_gap"),
                    task_id: TaskId::from("tsk_ready"),
                    run_id: None,
                    author_user_id: UserId::from("usr_owner"),
                    author_display_name: "Owner".to_string(),
                    body: "beyond gap".to_string(),
                    created_at: Timestamp(2_000),
                },
            },
            created_at: Timestamp(2_000),
        }]));
    }
    let after = q.after_cursor.unwrap_or(0);
    let limit = q.limit.unwrap_or(200);
    let out = data
        .events
        .iter()
        .filter(|e| e.cursor_seq.0 > after)
        .take(limit)
        .cloned()
        .collect();
    Ok(Json(out))
}

pub async fn mock_commands(
    AxumState(st): AxumState<MockTeamServerState>,
    headers: HeaderMap,
    AxumPath(wid): AxumPath<String>,
    Json(cmd): Json<TeamCommand>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    check_auth(&headers, &st)?;
    let mut data = st.inner.lock().await;
    data.commands_seen.push(cmd.clone());
    let payload_hash = sha256_hex(
        serde_json::to_string(&cmd.canonical_for_hash())
            .unwrap()
            .as_bytes(),
    );
    let cmd_id = cmd.command_id().0.clone();
    if let Some((stored_hash, stored_resp)) = data.commands_by_id.get(&cmd_id) {
        if stored_hash != &payload_hash {
            return Err((
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "command_payload_conflict",
                    "message": format!("command_id {cmd_id} already exists with different payload")
                })),
            ));
        }
        let mut replay = stored_resp.clone();
        replay["idempotent_replay"] = json!(true);
        return Ok(Json(replay));
    }

    let pid = data.snapshot.projects.first().map(|p| p.project_id.clone());
    let resp = match &cmd {
        TeamCommand::AddComment(c) => {
            data.cursor += 1;
            let seq = CursorSeq(data.cursor);
            let comment = TaskComment {
                id: c.comment_id.clone(),
                task_id: c.task_id.clone(),
                run_id: c.run_id.clone(),
                author_user_id: UserId::from("usr_owner"),
                author_display_name: "Owner".to_string(),
                body: c.body.clone(),
                created_at: Timestamp(3_000),
            };
            data.snapshot.comments.push(comment.clone());
            data.snapshot.cursor_seq = seq;
            data.events.push(WorkspaceEventEnvelope {
                workspace_id: WorkspaceId::from(wid),
                project_id: pid,
                aggregate_type: "task".to_string(),
                aggregate_id: c.task_id.0.clone(),
                actor_user_id: UserId::from("usr_owner"),
                cursor_seq: seq,
                event: WorkspaceSyncEvent::CommentAppended { comment },
                created_at: Timestamp(3_000),
            });
            json!({
                "command_id": c.command_id.0,
                "idempotent_replay": false,
                "cursor_seq": seq.0,
                "aggregate_type": "task",
                "aggregate_id": c.task_id.0,
                "revision": data.current_task_rev
            })
        }
        TeamCommand::UpdateTaskPlan(u) => handle_update_plan_cmd(&mut data, &wid, pid, u)?.0,
        TeamCommand::CreateTeamRun(r) => handle_create_team_run_cmd(&mut data, &wid, r)?,
        TeamCommand::ForceCancelUnreachableRun(c) => {
            data.cursor += 1;
            json!({
                "command_id": c.command_id.0,
                "idempotent_replay": false,
                "cursor_seq": data.cursor,
                "aggregate_type": "run",
                "aggregate_id": c.run_id.0,
                "revision": 2
            })
        }
        other => json!({
            "command_id": other.command_id().0,
            "idempotent_replay": false,
            "cursor_seq": data.cursor,
            "aggregate_type": "task",
            "aggregate_id": "tsk_ready",
            "revision": data.current_task_rev
        }),
    };
    data.commands_by_id
        .insert(cmd_id, (payload_hash, resp.clone()));
    Ok(Json(resp))
}

pub async fn spawn_daemon(
    db_path: &Path,
) -> (
    String,
    AppState,
    watch::Sender<bool>,
    tokio::task::JoinHandle<()>,
) {
    let conn = open_sqlite(db_path).unwrap();
    initialize_schema(&conn).unwrap();
    drop(conn);
    let state = AppState::new(db_path.to_path_buf());
    let (tx, _rx) = watch::channel(false);
    let router = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{addr}"), state, tx, handle)
}

fn handle_update_plan_cmd(
    data: &mut MockTeamData,
    wid: &str,
    pid: Option<runlark_domain::ProjectId>,
    u: &runlark_domain::UpdateTaskPlanCommand,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if u.expected_revision.0 != data.current_task_rev {
        return Err((
            StatusCode::CONFLICT,
            Json(json!({
                "error": "revision_conflict",
                "message": "plan base_revision mismatch",
                "current_revision": data.current_task_rev
            })),
        ));
    }
    data.current_task_rev += 1;
    data.cursor += 1;
    let seq = CursorSeq(data.cursor);
    let next_rev = data.current_task_rev;
    let updated_task = data
        .snapshot
        .tasks
        .iter_mut()
        .find(|t| t.id == u.task_id)
        .map(|task| {
            task.plan_text = Some(u.plan_text.clone());
            task.plan_version += 1;
            task.revision = Revision(next_rev);
            task.clone()
        });
    if let Some(task) = updated_task {
        data.events.push(WorkspaceEventEnvelope {
            workspace_id: WorkspaceId::from(wid),
            project_id: pid,
            aggregate_type: "task".to_string(),
            aggregate_id: u.task_id.0.clone(),
            actor_user_id: UserId::from("usr_owner"),
            cursor_seq: seq,
            event: WorkspaceSyncEvent::TaskUpserted { task },
            created_at: Timestamp(4_000),
        });
    }
    data.snapshot.cursor_seq = seq;
    Ok(Json(json!({
        "command_id": u.command_id.0,
        "idempotent_replay": false,
        "cursor_seq": seq.0,
        "aggregate_type": "task",
        "aggregate_id": u.task_id.0,
        "revision": data.current_task_rev
    })))
}

fn handle_create_team_run_cmd(
    data: &mut MockTeamData,
    wid: &str,
    c: &runlark_domain::CreateTeamRunCommand,
) -> Result<Value, (StatusCode, Json<Value>)> {
    let task_idx = data
        .snapshot
        .tasks
        .iter()
        .position(|t| t.id == c.task_id)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "not_found", "message": "task not found" })),
            )
        })?;
    let current_rev = data.snapshot.tasks[task_idx].revision;
    if current_rev != c.expected_revision {
        return Err((
            StatusCode::CONFLICT,
            Json(json!({
                "error": "revision_mismatch",
                "message": format!("expected {}, actual {}", c.expected_revision, current_rev),
                "current_revision": current_rev.0
            })),
        ));
    }
    if data.snapshot.tasks[task_idx].active_run_id.is_some() {
        return Err((
            StatusCode::CONFLICT,
            Json(json!({
                "error": "active_run_exists",
                "message": "task already has an active run",
                "current_revision": current_rev.0
            })),
        ));
    }

    let updated_task = {
        let task = &mut data.snapshot.tasks[task_idx];
        task.stage = TaskStage::InProgress;
        task.active_run_id = Some(c.run_id.clone());
        task.latest_run_id = Some(c.run_id.clone());
        task.revision = Revision(task.revision.0 + 1);
        task.clone()
    };
    let run = Run {
        id: c.run_id.clone(),
        session_id: updated_task
            .active_session_id
            .clone()
            .unwrap_or_else(|| SessionId::from(format!("sess_{}", c.run_id.0))),
        status: RunStatus::Queued,
        worktree_path: String::new(),
        revision: Revision(1),
        created_at: Timestamp(4_000),
        updated_at: Timestamp(4_000),
        task_id: Some(c.task_id.clone()),
        project_id: Some(c.project_id.clone()),
        workflow_stage: WorkflowStage::Implementation,
        task_kind: updated_task.kind,
        workflow_digest: None,
        base_sha: None,
        head_sha: None,
        plan_version: updated_task.plan_version,
        policy_digest: Some(PolicyDigest::from("pol_mock")),
        budget_policy: BudgetPolicy::default(),
        cost_summary: CostSummary::default(),
        repair_attempts: 0,
        pr_url: None,
    };
    data.snapshot.runs.push(TeamRunEntry {
        run: run.clone(),
        ownership: RunOwnership::default(),
    });
    let run_seq = emit_created_team_run_events(data, wid, &c.project_id, updated_task, &run);
    Ok(json!({
        "command_id": c.command_id.0,
        "idempotent_replay": false,
        "cursor_seq": run_seq.0,
        "aggregate_type": "run",
        "aggregate_id": run.id.0,
        "revision": run.revision.0
    }))
}

fn emit_created_team_run_events(
    data: &mut MockTeamData,
    wid: &str,
    project_id: &runlark_domain::ProjectId,
    updated_task: runlark_domain::Task,
    run: &Run,
) -> CursorSeq {
    data.cursor += 1;
    let task_seq = CursorSeq(data.cursor);
    data.events.push(WorkspaceEventEnvelope {
        workspace_id: WorkspaceId::from(wid),
        project_id: Some(project_id.clone()),
        aggregate_type: "task".to_string(),
        aggregate_id: updated_task.id.0.clone(),
        actor_user_id: UserId::from("usr_owner"),
        cursor_seq: task_seq,
        event: WorkspaceSyncEvent::TaskUpserted { task: updated_task },
        created_at: Timestamp(4_000),
    });
    data.cursor += 1;
    let run_seq = CursorSeq(data.cursor);
    data.snapshot.cursor_seq = run_seq;
    data.events.push(WorkspaceEventEnvelope {
        workspace_id: WorkspaceId::from(wid),
        project_id: Some(project_id.clone()),
        aggregate_type: "run".to_string(),
        aggregate_id: run.id.0.clone(),
        actor_user_id: UserId::from("usr_owner"),
        cursor_seq: run_seq,
        event: WorkspaceSyncEvent::RunUpserted {
            run: run.clone(),
            ownership: RunOwnership::default(),
        },
        created_at: Timestamp(4_000),
    });
    run_seq
}
