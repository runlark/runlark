use crate::routes::error::{ApiError, api_error, storage_error_to_http, team_server_error_to_http};
use crate::routes::team_gate;
use crate::state::AppState;
use crate::team_sync::{TeamServerError, TeamSyncWorker};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use runlark_domain::{
    AddCommentCommand, CommandId, CommentId, DraftId, DraftKind, DraftSyncState, OfflineDraft,
    ProjectId, RunId, TaskComment, TaskId, TeamCommand, Timestamp, UserId,
};
use runlark_local::{CommentStore, DraftStore, SyncOutboxStore, TaskStore, with_immediate_tx};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct AddCommentRequest {
    pub command_id: Option<String>,
    pub comment_id: Option<String>,
    pub run_id: Option<String>,
    pub body: String,
    pub author_user_id: Option<String>,
    pub author_display_name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "delivery", rename_all = "snake_case")]
pub enum AddCommentResponse {
    Synced { comment: TaskComment },
    QueuedOffline { draft: OfflineDraft },
}

/// Lists immutable append-only comments on `task_id`.
///
/// # Errors
/// Returns `404` if task missing.
pub async fn list_task_comments(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<TaskComment>>, ApiError> {
    let task_id = TaskId::from(id);
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    if TaskStore::get_task(&conn, &task_id)
        .map_err(storage_error_to_http)?
        .is_none()
    {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "task_not_found",
            format!("Task not found: {task_id}"),
        ));
    }
    let comments =
        CommentStore::list_comments_for_task(&conn, &task_id).map_err(storage_error_to_http)?;
    Ok(Json(comments))
}

/// Appends an immutable comment to `task_id`. For `TeamManaged` projects, posts to the server
/// and queues an explicit `OfflineDraft` + `sync_outbox` entry if the server is unreachable.
///
/// # Errors
/// Returns `400` if body blank, `404` if task missing, or server rejection (`403`/`409`).
pub async fn add_task_comment(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<AddCommentRequest>,
) -> Result<(StatusCode, Json<AddCommentResponse>), ApiError> {
    if req.body.trim().is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_comment",
            "comment body must not be empty",
        ));
    }
    let task_id = TaskId::from(id);
    let now = Timestamp(AppState::current_timestamp_ms());
    let command_id = CommandId::from(
        req.command_id
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("cmd_cmt_{}", Uuid::new_v4().simple())),
    );
    let comment_id = CommentId::from(
        req.comment_id
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("cmt_{}", Uuid::new_v4().simple())),
    );
    let run_id = req.run_id.filter(|s| !s.trim().is_empty()).map(RunId::from);

    if let Some((task, ctx)) = team_gate::resolve_task_team_context(&state, &task_id)? {
        return submit_or_queue_team_comment(
            &state,
            &ctx,
            TeamCommentParams {
                project_id: task.project_id,
                task_id,
                comment_id,
                command_id,
                run_id,
                body: req.body,
            },
            now,
        )
        .await;
    }

    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    if TaskStore::get_task(&conn, &task_id)
        .map_err(storage_error_to_http)?
        .is_none()
    {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "task_not_found",
            format!("Task not found: {task_id}"),
        ));
    }
    let comment = TaskComment {
        id: comment_id,
        task_id,
        run_id,
        author_user_id: UserId::from(
            req.author_user_id
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "local".to_string()),
        ),
        author_display_name: req
            .author_display_name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Human".to_string()),
        body: req.body,
        created_at: now,
    };
    CommentStore::upsert_comment(&conn, &comment).map_err(storage_error_to_http)?;
    Ok((
        StatusCode::CREATED,
        Json(AddCommentResponse::Synced { comment }),
    ))
}

struct TeamCommentParams {
    project_id: ProjectId,
    task_id: TaskId,
    comment_id: CommentId,
    command_id: CommandId,
    run_id: Option<RunId>,
    body: String,
}

async fn submit_or_queue_team_comment(
    state: &AppState,
    ctx: &team_gate::TeamProjectContext,
    params: TeamCommentParams,
    now: Timestamp,
) -> Result<(StatusCode, Json<AddCommentResponse>), ApiError> {
    let cmd = TeamCommand::AddComment(AddCommentCommand {
        command_id: params.command_id,
        comment_id: params.comment_id.clone(),
        task_id: params.task_id.clone(),
        run_id: params.run_id.clone(),
        body: params.body.clone(),
        created_at: now,
    });
    let worker = TeamSyncWorker::new(state.clone());
    match worker
        .submit_command_and_sync(&ctx.server_url, &ctx.bearer_token, &ctx.workspace_id, &cmd)
        .await
    {
        Ok(_) => {
            let conn = state.get_connection().map_err(|e| {
                api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    e.to_string(),
                )
            })?;
            let comment = CommentStore::get_comment(&conn, &params.comment_id)
                .map_err(storage_error_to_http)?
                .ok_or_else(|| {
                    api_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "storage_error",
                        "comment missing after sync",
                    )
                })?;
            Ok((
                StatusCode::CREATED,
                Json(AddCommentResponse::Synced { comment }),
            ))
        }
        Err(TeamServerError::Offline(_)) => {
            let draft = OfflineDraft {
                id: DraftId::from(format!("draft_cmt_{}", Uuid::new_v4().simple())),
                project_id: params.project_id,
                task_id: Some(params.task_id),
                kind: DraftKind::TaskComment,
                payload_json: serde_json::json!({
                    "comment_id": params.comment_id.0,
                    "run_id": params.run_id.map(|r| r.0),
                    "body": params.body,
                })
                .to_string(),
                base_revision: None,
                sync_state: DraftSyncState::PendingSync,
                created_at: now,
                updated_at: now,
            };
            let outbox = TeamSyncWorker::build_outbox_entry(
                &ctx.workspace_id,
                Some(&draft.project_id),
                Some(&draft.id),
                &cmd,
                None,
                now,
            )
            .map_err(team_server_error_to_http)?;
            let mut conn = state.get_connection().map_err(|e| {
                api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    e.to_string(),
                )
            })?;
            with_immediate_tx::<_, runlark_local::StorageError, _>(&mut conn, |tx| {
                DraftStore::save_draft(tx, &draft)?;
                SyncOutboxStore::enqueue(tx, &outbox)?;
                Ok(())
            })
            .map_err(storage_error_to_http)?;
            Ok((
                StatusCode::ACCEPTED,
                Json(AddCommentResponse::QueuedOffline { draft }),
            ))
        }
        Err(err) => Err(team_server_error_to_http(err)),
    }
}
