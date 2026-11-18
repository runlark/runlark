use crate::auth::AuthenticatedUser;
use crate::error::ServerError;
use crate::pg::codec::{row_to_task, task_kind_to_str, task_stage_to_str, workflow_stage_to_str};
use crate::pg::commands::{TeamCommandResult, WorkspaceEventRecord, emit_workspace_event};
use runlark_domain::{
    CreateTaskCommand, MoveTaskStageCommand, Revision, Task, TaskStage, Timestamp,
    UpdateTaskPlanCommand, WorkspaceId, WorkspaceSyncEvent, validate_task_stage_move,
};

pub(crate) async fn exec_create_task(
    tx: &tokio_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    cmd: CreateTaskCommand,
    now: Timestamp,
    now_i64: i64,
) -> Result<TeamCommandResult, ServerError> {
    let proj_exists = tx
        .query_opt(
            "SELECT 1 FROM team_projects WHERE project_id = $1 AND workspace_id = $2",
            &[&cmd.project_id.as_ref(), &workspace_id.as_ref()],
        )
        .await?;
    if proj_exists.is_none() {
        return Err(ServerError::NotFound(format!(
            "project not found in workspace: {}",
            cmd.project_id
        )));
    }
    let existing = tx
        .query_opt(
            "SELECT 1 FROM team_tasks WHERE task_id = $1",
            &[&cmd.task_id.as_ref()],
        )
        .await?;
    if existing.is_some() {
        return Err(ServerError::conflict(
            "task_already_exists",
            format!("task {} already exists", cmd.task_id),
            None,
        ));
    }

    let wf_stage = cmd.kind.initial_workflow_stage();
    let budget_json = cmd
        .budget_override
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;

    tx.execute(
        concat!(
            "INSERT INTO team_tasks ",
            "(task_id, workspace_id, project_id, title, description, kind, stage, workflow_stage, ",
            "plan_text, plan_version, base_ref, head_ref, pr_url, active_session_id, ",
            "active_run_id, latest_run_id, budget_override_json, revision, created_at, updated_at) ",
            "VALUES ($1, $2, $3, $4, $5, $6, 'inbox', $7, NULL, 1, $8, NULL, NULL, NULL, NULL, NULL, $9, 1, $10, $10)",
        ),
        &[
            &cmd.task_id.as_ref(),
            &workspace_id.as_ref(),
            &cmd.project_id.as_ref(),
            &cmd.title,
            &cmd.description,
            &task_kind_to_str(cmd.kind),
            &workflow_stage_to_str(wf_stage),
            &cmd.base_ref,
            &budget_json,
            &now_i64,
        ],
    )
    .await?;

    let task = Task {
        id: cmd.task_id.clone(),
        project_id: cmd.project_id.clone(),
        title: cmd.title,
        description: cmd.description,
        kind: cmd.kind,
        stage: TaskStage::Inbox,
        workflow_stage: wf_stage,
        plan_text: None,
        plan_version: 1,
        base_ref: cmd.base_ref,
        head_ref: None,
        pr_url: None,
        active_session_id: None,
        active_run_id: None,
        latest_run_id: None,
        budget_override: cmd.budget_override,
        revision: Revision(1),
        created_at: now,
        updated_at: now,
    };
    let cursor = emit_workspace_event(
        tx,
        WorkspaceEventRecord {
            workspace_id,
            project_id: Some(task.project_id.as_ref()),
            aggregate_type: "task",
            aggregate_id: task.id.as_ref(),
            event_type: "task_upserted",
            event: &WorkspaceSyncEvent::TaskUpserted { task: task.clone() },
            actor_user_id: actor.user_id.as_ref(),
            created_at: now_i64,
        },
    )
    .await?;

    Ok(TeamCommandResult {
        command_id: cmd.command_id.0,
        idempotent_replay: false,
        cursor_seq: cursor,
        aggregate_type: "task".to_string(),
        aggregate_id: task.id.0,
        revision: 1,
    })
}

pub(crate) async fn exec_move_task_stage(
    tx: &tokio_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    cmd: MoveTaskStageCommand,
    now: Timestamp,
    now_i64: i64,
) -> Result<TeamCommandResult, ServerError> {
    let row = tx
        .query_opt(
            concat!(
                "SELECT task_id, project_id, title, description, kind, stage, workflow_stage, ",
                "plan_text, plan_version, base_ref, head_ref, pr_url, active_session_id, ",
                "active_run_id, latest_run_id, budget_override_json, revision, created_at, updated_at ",
                "FROM team_tasks WHERE task_id = $1 AND workspace_id = $2 FOR UPDATE",
            ),
            &[&cmd.task_id.as_ref(), &workspace_id.as_ref()],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound(format!("task not found: {}", cmd.task_id)))?;
    let mut task = row_to_task(&row)?;
    if task.revision != cmd.expected_revision {
        return Err(ServerError::conflict(
            "revision_mismatch",
            format!(
                "task {} expected revision {}, actual {}",
                task.id, cmd.expected_revision.0, task.revision.0
            ),
            Some(task.revision.0),
        ));
    }

    let has_executing = if let Some(ref active_rid) = task.active_run_id {
        let r = tx
            .query_opt(
                "SELECT status FROM team_runs WHERE run_id = $1",
                &[&active_rid.as_ref()],
            )
            .await?;
        r.is_some_and(|row| {
            let st: String = row.get("status");
            st == "running" || st == "cancel_requested"
        })
    } else {
        false
    };

    let has_accept = tx
        .query_opt(
            "SELECT 1 FROM team_decisions WHERE task_id = $1 AND kind = 'accept' LIMIT 1",
            &[&task.id.as_ref()],
        )
        .await?
        .is_some();

    validate_task_stage_move(
        &task,
        cmd.target_stage,
        &cmd.actor,
        has_executing,
        has_accept,
    )?;

    let next_rev = Revision(task.revision.0.saturating_add(1));
    tx.execute(
        concat!(
            "UPDATE team_tasks SET stage = $3, revision = $4, updated_at = $5 ",
            "WHERE task_id = $1 AND workspace_id = $2",
        ),
        &[
            &task.id.as_ref(),
            &workspace_id.as_ref(),
            &task_stage_to_str(cmd.target_stage),
            &ServerError::to_pg_i64(next_rev.0)?,
            &now_i64,
        ],
    )
    .await?;

    task.stage = cmd.target_stage;
    task.revision = next_rev;
    task.updated_at = now;
    let cursor = emit_workspace_event(
        tx,
        WorkspaceEventRecord {
            workspace_id,
            project_id: Some(task.project_id.as_ref()),
            aggregate_type: "task",
            aggregate_id: task.id.as_ref(),
            event_type: "task_upserted",
            event: &WorkspaceSyncEvent::TaskUpserted { task: task.clone() },
            actor_user_id: actor.user_id.as_ref(),
            created_at: now_i64,
        },
    )
    .await?;

    Ok(TeamCommandResult {
        command_id: cmd.command_id.0,
        idempotent_replay: false,
        cursor_seq: cursor,
        aggregate_type: "task".to_string(),
        aggregate_id: task.id.0,
        revision: next_rev.0,
    })
}

pub(crate) async fn exec_update_task_plan(
    tx: &tokio_postgres::Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    cmd: UpdateTaskPlanCommand,
    now: Timestamp,
    now_i64: i64,
) -> Result<TeamCommandResult, ServerError> {
    let row = tx
        .query_opt(
            concat!(
                "SELECT task_id, project_id, title, description, kind, stage, workflow_stage, ",
                "plan_text, plan_version, base_ref, head_ref, pr_url, active_session_id, ",
                "active_run_id, latest_run_id, budget_override_json, revision, created_at, updated_at ",
                "FROM team_tasks WHERE task_id = $1 AND workspace_id = $2 FOR UPDATE",
            ),
            &[&cmd.task_id.as_ref(), &workspace_id.as_ref()],
        )
        .await?
        .ok_or_else(|| ServerError::NotFound(format!("task not found: {}", cmd.task_id)))?;
    let mut task = row_to_task(&row)?;
    if task.revision != cmd.expected_revision {
        return Err(ServerError::conflict(
            "revision_mismatch",
            format!(
                "task {} expected revision {}, actual {}",
                task.id, cmd.expected_revision.0, task.revision.0
            ),
            Some(task.revision.0),
        ));
    }

    let next_plan_ver = task.plan_version.saturating_add(1);
    let next_rev = Revision(task.revision.0.saturating_add(1));
    tx.execute(
        concat!(
            "UPDATE team_tasks SET plan_text = $3, plan_version = $4, revision = $5, updated_at = $6 ",
            "WHERE task_id = $1 AND workspace_id = $2",
        ),
        &[
            &task.id.as_ref(),
            &workspace_id.as_ref(),
            &cmd.plan_text,
            &ServerError::to_pg_i32(next_plan_ver)?,
            &ServerError::to_pg_i64(next_rev.0)?,
            &now_i64,
        ],
    )
    .await?;

    task.plan_text = Some(cmd.plan_text);
    task.plan_version = next_plan_ver;
    task.revision = next_rev;
    task.updated_at = now;
    let cursor = emit_workspace_event(
        tx,
        WorkspaceEventRecord {
            workspace_id,
            project_id: Some(task.project_id.as_ref()),
            aggregate_type: "task",
            aggregate_id: task.id.as_ref(),
            event_type: "task_upserted",
            event: &WorkspaceSyncEvent::TaskUpserted { task: task.clone() },
            actor_user_id: actor.user_id.as_ref(),
            created_at: now_i64,
        },
    )
    .await?;

    Ok(TeamCommandResult {
        command_id: cmd.command_id.0,
        idempotent_replay: false,
        cursor_seq: cursor,
        aggregate_type: "task".to_string(),
        aggregate_id: task.id.0,
        revision: next_rev.0,
    })
}
