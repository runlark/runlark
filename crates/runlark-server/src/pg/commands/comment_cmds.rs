use crate::auth::AuthenticatedUser;
use crate::error::ServerError;
use crate::pg::commands::{TeamCommandResult, WorkspaceEventRecord, emit_workspace_event};
use runlark_domain::{AddCommentCommand, TaskComment, Timestamp, WorkspaceId, WorkspaceSyncEvent};

pub(crate) async fn exec_add_comment(
    tx: &tokio_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    command_id: &str,
    cmd: AddCommentCommand,
    now: Timestamp,
    now_i64: i64,
) -> Result<TeamCommandResult, ServerError> {
    if cmd.body.trim().is_empty() {
        return Err(ServerError::BadRequest(
            "comment body must not be empty".to_string(),
        ));
    }
    let dup = tx
        .query_opt(
            "SELECT 1 FROM team_comments WHERE comment_id = $1",
            &[&cmd.comment_id.as_ref()],
        )
        .await?;
    if dup.is_some() {
        return Err(ServerError::conflict(
            "comment_already_exists",
            format!("comment {} already exists", cmd.comment_id),
            None,
        ));
    }

    let task_row = tx
        .query_opt(
            "SELECT project_id FROM team_tasks WHERE task_id = $1 AND workspace_id = $2",
            &[&cmd.task_id.as_ref(), &workspace_id.as_ref()],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound(format!("task not found: {}", cmd.task_id)))?;
    let project_id: String = task_row.get("project_id");

    if let Some(ref rid) = cmd.run_id {
        let run_ok = tx
            .query_opt(
                "SELECT 1 FROM team_runs WHERE run_id = $1 AND workspace_id = $2 AND task_id = $3",
                &[&rid.as_ref(), &workspace_id.as_ref(), &cmd.task_id.as_ref()],
            )
            .await?;
        if run_ok.is_none() {
            return Err(ServerError::NotFound(format!(
                "run {rid} not found on task {}",
                cmd.task_id
            )));
        }
    }

    tx.execute(
        concat!(
            "INSERT INTO team_comments ",
            "(comment_id, workspace_id, task_id, run_id, author_user_id, author_display_name, body, created_at) ",
            "VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        ),
        &[
            &cmd.comment_id.as_ref(),
            &workspace_id.as_ref(),
            &cmd.task_id.as_ref(),
            &cmd.run_id.as_ref().map(|r| r.0.as_str()),
            &actor.user_id.as_ref(),
            &actor.display_name,
            &cmd.body,
            &now_i64,
        ],
    )
    .await?;

    let comment = TaskComment {
        id: cmd.comment_id.clone(),
        task_id: cmd.task_id,
        run_id: cmd.run_id,
        author_user_id: actor.user_id.clone(),
        author_display_name: actor.display_name.clone(),
        body: cmd.body,
        created_at: now,
    };
    let cursor = emit_workspace_event(
        tx,
        WorkspaceEventRecord {
            workspace_id,
            project_id: Some(&project_id),
            aggregate_type: "comment",
            aggregate_id: comment.id.as_ref(),
            event_type: "comment_appended",
            event: &WorkspaceSyncEvent::CommentAppended {
                comment: comment.clone(),
            },
            actor_user_id: actor.user_id.as_ref(),
            created_at: now_i64,
        },
    )
    .await?;

    Ok(TeamCommandResult {
        command_id: command_id.to_string(),
        idempotent_replay: false,
        cursor_seq: cursor,
        aggregate_type: "comment".to_string(),
        aggregate_id: comment.id.0,
        revision: 1,
    })
}
