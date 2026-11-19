use crate::routes::error::{ApiError, api_error, storage_error_to_http, team_server_error_to_http};
use crate::routes::team::is_project_team_managed;
use crate::routes::team_workspaces::resolve_auth;
use crate::state::AppState;
use crate::team_sync::TeamSyncWorker;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use runlark_domain::{
    AddCommentCommand, CommandId, CommentId, CreateTaskCommand, DraftId, DraftKind, DraftSyncState,
    OfflineDraft, ProjectId, Revision, RunId, TaskId, TaskKind, TeamCommand, Timestamp,
    UpdateTaskPlanCommand,
};
use runlark_local::{DraftStore, SyncOutboxEntry, SyncOutboxStore, with_immediate_tx};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize, Default)]
pub struct DraftsQuery {
    pub project_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DraftsListResponse {
    pub drafts: Vec<OfflineDraft>,
    pub outbox: Vec<SyncOutboxEntry>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDraftRequest {
    pub draft_id: Option<String>,
    pub project_id: String,
    pub task_id: Option<String>,
    pub kind: DraftKind,
    pub payload_json: String,
    pub base_revision: Option<u64>,
    #[serde(default = "default_true")]
    pub queue_sync: bool,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, Default)]
pub struct RetryDraftRequest {
    pub expected_revision: Option<u64>,
    pub payload_json: Option<String>,
}

/// Lists persisted offline drafts and `sync_outbox` entries.
///
/// # Errors
/// Returns `500` on `SQLite` failure.
pub async fn list_drafts(
    State(state): State<AppState>,
    Query(query): Query<DraftsQuery>,
) -> Result<Json<DraftsListResponse>, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let pid = query
        .project_id
        .filter(|s| !s.trim().is_empty())
        .map(ProjectId::from);
    let drafts = DraftStore::list_drafts(&conn, pid.as_ref()).map_err(storage_error_to_http)?;
    let outbox = SyncOutboxStore::list_entries(&conn, None).map_err(storage_error_to_http)?;
    Ok(Json(DraftsListResponse { drafts, outbox }))
}

/// Persists an explicit [`OfflineDraft`] (and queues it in `sync_outbox` when `queue_sync` is
/// `true` for a `TeamManaged` project).
///
/// # Errors
/// Returns `400` on invalid payload, `500` on `SQLite` failure.
pub async fn create_draft(
    State(state): State<AppState>,
    Json(req): Json<CreateDraftRequest>,
) -> Result<(StatusCode, Json<OfflineDraft>), ApiError> {
    let project_id = ProjectId::from(req.project_id);
    let task_id = req
        .task_id
        .filter(|s| !s.trim().is_empty())
        .map(TaskId::from);
    let draft_id = DraftId::from(
        req.draft_id
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("draft_{}", Uuid::new_v4().simple())),
    );
    let now = Timestamp(AppState::current_timestamp_ms());
    let team_binding = is_project_team_managed(&state, &project_id)?;
    let queue_for_sync =
        req.queue_sync && team_binding.is_some() && req.kind != DraftKind::DecisionIntent;
    let sync_state = if queue_for_sync {
        DraftSyncState::PendingSync
    } else {
        DraftSyncState::LocalDraft
    };
    let draft = OfflineDraft {
        id: draft_id,
        project_id: project_id.clone(),
        task_id,
        kind: req.kind,
        payload_json: req.payload_json,
        base_revision: req.base_revision.map(Revision),
        sync_state,
        created_at: now,
        updated_at: now,
    };

    let mut conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    if let Some((wid, _)) = team_binding
        && queue_for_sync
        && let Some(cmd) = build_command_for_draft(&draft, None, now)?
    {
        let outbox = TeamSyncWorker::build_outbox_entry(
            &wid,
            Some(&project_id),
            Some(&draft.id),
            &cmd,
            draft.base_revision,
            now,
        )
        .map_err(team_server_error_to_http)?;
        with_immediate_tx::<_, runlark_local::StorageError, _>(&mut conn, |tx| {
            DraftStore::save_draft(tx, &draft)?;
            SyncOutboxStore::enqueue(tx, &outbox)?;
            Ok(())
        })
        .map_err(storage_error_to_http)?;
        state.notify_team_sync();
    } else {
        DraftStore::save_draft(&conn, &draft).map_err(storage_error_to_http)?;
    }

    Ok((StatusCode::CREATED, Json(draft)))
}

/// Retries a conflicted or pending [`OfflineDraft`] with a fresh `CommandId` and optional
/// updated `expected_revision` / `payload_json`.
///
/// # Errors
/// Returns `404` if draft missing, `400` if payload cannot map to a command.
pub async fn retry_draft(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<RetryDraftRequest>>,
) -> Result<Json<OfflineDraft>, ApiError> {
    let draft_id = DraftId::from(id);
    let req = body.map(|b| b.0).unwrap_or_default();
    let now = Timestamp(AppState::current_timestamp_ms());

    let (wid, server_url, outbox_entry) = prepare_retry_outbox(&state, &draft_id, &req, now)?;
    {
        let mut conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        with_immediate_tx::<_, runlark_local::StorageError, _>(&mut conn, |tx| {
            DraftStore::mark_pending_sync(
                tx,
                &draft_id,
                req.expected_revision.map(Revision),
                req.payload_json.as_deref(),
                now,
            )?;
            SyncOutboxStore::delete_for_draft(tx, &draft_id)?;
            if let Some(ref entry) = outbox_entry {
                SyncOutboxStore::enqueue(tx, entry)?;
            }
            Ok(())
        })
        .map_err(storage_error_to_http)?;
    }

    if outbox_entry.is_some()
        && let Ok(auth) = resolve_auth(&state, Some(&server_url), Some(&wid))
    {
        let _ = TeamSyncWorker::new(state.clone())
            .sync_workspace(&auth.server_url, &auth.bearer_token, &wid)
            .await;
    }

    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let latest = DraftStore::get_draft(&conn, &draft_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "draft_not_found",
                format!("Offline draft not found: {draft_id}"),
            )
        })?;
    Ok(Json(latest))
}

fn prepare_retry_outbox(
    state: &AppState,
    draft_id: &DraftId,
    req: &RetryDraftRequest,
    now: Timestamp,
) -> Result<(runlark_domain::WorkspaceId, String, Option<SyncOutboxEntry>), ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let mut draft = DraftStore::get_draft(&conn, draft_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "draft_not_found",
                format!("Offline draft not found: {draft_id}"),
            )
        })?;
    drop(conn);
    if matches!(draft.sync_state, DraftSyncState::Synced) {
        return Err(api_error(
            StatusCode::CONFLICT,
            "draft_already_synced",
            format!("Offline draft {draft_id} is already synced and cannot be retried"),
        ));
    }
    if draft.kind == DraftKind::DecisionIntent {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "decision_intent_not_syncable",
            "DecisionIntent drafts require online confirmation and cannot be queued in sync_outbox",
        ));
    }
    let Some((wid, url)) = is_project_team_managed(state, &draft.project_id)? else {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "solo_project_draft",
            "Cannot sync draft on a SoloLocal project",
        ));
    };

    if let Some(rev) = req.expected_revision {
        draft.base_revision = Some(Revision(rev));
    }
    if let Some(ref payload) = req.payload_json {
        draft.payload_json.clone_from(payload);
    }
    let cmd_id = CommandId::from(format!("cmd_retry_{}", Uuid::new_v4().simple()));
    let maybe_outbox = build_command_for_draft(&draft, Some(cmd_id), now)?
        .map(|cmd| {
            TeamSyncWorker::build_outbox_entry(
                &wid,
                Some(&draft.project_id),
                Some(&draft.id),
                &cmd,
                draft.base_revision,
                now,
            )
        })
        .transpose()
        .map_err(team_server_error_to_http)?;
    Ok((wid, url, maybe_outbox))
}

/// Discards an unresolved [`OfflineDraft`] and removes its pending/conflicted outbox entries.
///
/// # Errors
/// Returns `404` if draft missing.
pub async fn delete_draft(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let draft_id = DraftId::from(id);
    let mut conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    with_immediate_tx::<_, runlark_local::StorageError, _>(&mut conn, |tx| {
        SyncOutboxStore::delete_for_draft(tx, &draft_id)?;
        DraftStore::delete_draft(tx, &draft_id)?;
        Ok(())
    })
    .map_err(storage_error_to_http)?;
    Ok(StatusCode::NO_CONTENT)
}

fn draft_req_task_id(draft: &OfflineDraft, label: &str) -> Result<TaskId, ApiError> {
    draft.task_id.clone().ok_or_else(|| {
        api_error(
            StatusCode::BAD_REQUEST,
            "invalid_draft_payload",
            format!("{label} draft requires task_id"),
        )
    })
}

fn draft_req_str(val: &serde_json::Value, key: &str, label: &str) -> Result<String, ApiError> {
    val[key].as_str().map(str::to_string).ok_or_else(|| {
        api_error(
            StatusCode::BAD_REQUEST,
            "invalid_draft_payload",
            format!("{label} draft requires {key}"),
        )
    })
}

fn build_comment_draft_cmd(
    draft: &OfflineDraft,
    val: &serde_json::Value,
    command_id: CommandId,
    now: Timestamp,
) -> Result<TeamCommand, ApiError> {
    let task_id = draft_req_task_id(draft, "TaskComment")?;
    let body = draft_req_str(val, "body", "TaskComment")?;
    let comment_id = val["comment_id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(
            || CommentId::from(format!("cmt_{}", draft.id.0)),
            CommentId::from,
        );
    let run_id = val["run_id"].as_str().map(RunId::from);
    Ok(TeamCommand::AddComment(AddCommentCommand {
        command_id,
        comment_id,
        task_id,
        run_id,
        body,
        created_at: now,
    }))
}

fn build_command_for_draft(
    draft: &OfflineDraft,
    explicit_cmd_id: Option<CommandId>,
    now: Timestamp,
) -> Result<Option<TeamCommand>, ApiError> {
    let val: serde_json::Value = serde_json::from_str(&draft.payload_json).map_err(|e| {
        api_error(
            StatusCode::BAD_REQUEST,
            "invalid_draft_payload",
            format!("draft payload must be JSON: {e}"),
        )
    })?;
    let cmd_id = explicit_cmd_id
        .unwrap_or_else(|| CommandId::from(format!("cmd_draft_{}", Uuid::new_v4().simple())));
    match draft.kind {
        DraftKind::TaskComment => build_comment_draft_cmd(draft, &val, cmd_id, now).map(Some),
        DraftKind::PlanEdit => Ok(Some(TeamCommand::UpdateTaskPlan(UpdateTaskPlanCommand {
            command_id: cmd_id,
            task_id: draft_req_task_id(draft, "PlanEdit")?,
            plan_text: draft_req_str(&val, "plan_text", "PlanEdit")?,
            expected_revision: draft.base_revision.unwrap_or(Revision(1)),
            created_at: now,
        }))),
        DraftKind::TaskCreate => {
            let title = draft_req_str(&val, "title", "TaskCreate")?;
            let description = val["description"].as_str().unwrap_or("").to_string();
            let task_id = val["task_id"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map_or_else(
                    || TaskId::from(format!("task_{}", draft.id.0)),
                    TaskId::from,
                );
            let kind = match val["kind"].as_str() {
                Some("quick_fix") => TaskKind::QuickFix,
                _ => TaskKind::Standard,
            };
            Ok(Some(TeamCommand::CreateTask(CreateTaskCommand {
                command_id: cmd_id,
                task_id,
                project_id: draft.project_id.clone(),
                title,
                description,
                kind,
                base_ref: val["base_ref"].as_str().map(str::to_string),
                budget_override: None,
                created_at: now,
            })))
        }
        DraftKind::DecisionIntent => Ok(None),
    }
}
