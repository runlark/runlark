use crate::auth::{AuthenticatedUser, authorize_member};
use crate::error::ServerError;
use crate::pg::codec::{
    decision_kind_to_str, run_status_to_str, task_kind_to_str, task_stage_to_str,
    workflow_stage_to_str,
};
use crate::pg::commands::{WorkspaceEventRecord, emit_workspace_event};
use crate::pg::{get_client, lock_workspace_write};
use deadpool_postgres::{Pool, Transaction};
use runlark_domain::{
    CursorSeq, Decision, ProjectId, Run, RunOwnership, ServerPromotionAck, Task, TeamProjectSpec,
    Timestamp, WorkspaceAction, WorkspaceId, WorkspaceSyncEvent, validate_solo_to_team_promotion,
};
use std::collections::HashSet;

struct PromotionTxContext<'a> {
    workspace_id: &'a WorkspaceId,
    actor: &'a AuthenticatedUser,
    project_id: &'a ProjectId,
    promoted_task_ids: &'a HashSet<runlark_domain::TaskId>,
    promoted_run_ids: &'a HashSet<runlark_domain::RunId>,
    now_i64: i64,
}

/// Sanitized solo project payload promoted into team workspace ownership.
#[derive(Debug, Clone)]
pub struct PromoteProjectPayload {
    pub project: TeamProjectSpec,
    pub tasks: Vec<Task>,
    pub runs: Vec<Run>,
    pub decisions: Vec<Decision>,
}

/// Promotes a solo-local project snapshot into a server-coordinated team project,
/// enforcing zero active/queued runs and returning an explicit [`ServerPromotionAck`].
///
/// # Errors
/// Returns [`ServerError`] on RBAC denial, active runs blocking promotion, or `PostgreSQL` conflict.
pub async fn promote_solo_project(
    pool: &Pool,
    actor: &AuthenticatedUser,
    workspace_id: &WorkspaceId,
    payload: PromoteProjectPayload,
    now: Timestamp,
) -> Result<ServerPromotionAck, ServerError> {
    let PromoteProjectPayload {
        project,
        tasks,
        runs,
        decisions,
    } = payload;
    if &project.workspace_id != workspace_id {
        return Err(ServerError::BadRequest(
            "project.workspace_id does not match URL workspace_id".to_string(),
        ));
    }
    validate_solo_to_team_promotion(&runs)?;

    let mut client = get_client(pool).await?;
    let tx = client.transaction().await?;
    lock_workspace_write(&tx, workspace_id).await?;
    authorize_member(
        &tx,
        workspace_id,
        &actor.user_id,
        WorkspaceAction::PromoteProject,
    )
    .await?;
    let existing = tx
        .query_opt(
            "SELECT workspace_id FROM team_projects WHERE project_id = $1 FOR UPDATE",
            &[&project.project_id.as_ref()],
        )
        .await?;
    if let Some(row) = existing {
        let existing_ws: String = row.get("workspace_id");
        return Err(ServerError::conflict(
            "project_already_promoted",
            format!(
                "project {} is already managed in workspace {existing_ws}",
                project.project_id
            ),
            None,
        ));
    }

    let now_i64 = ServerError::to_pg_i64(now.0)?;
    let epoch = project.ownership_epoch.max(1);
    let mut last_cursor =
        insert_promoted_project(&tx, workspace_id, actor, &project, epoch, now_i64).await?;

    let mut promoted_task_ids = HashSet::with_capacity(tasks.len());
    for task in tasks {
        promoted_task_ids.insert(task.id.clone());
        last_cursor =
            insert_promoted_task(&tx, workspace_id, actor, &project.project_id, task, now_i64)
                .await?;
    }

    let mut promoted_run_ids = HashSet::with_capacity(runs.len());
    for run in runs {
        promoted_run_ids.insert(run.id.clone());
        last_cursor = insert_promoted_run(
            &tx,
            workspace_id,
            actor,
            &project.project_id,
            run,
            &promoted_task_ids,
            now_i64,
        )
        .await?;
    }

    let ctx = PromotionTxContext {
        workspace_id,
        actor,
        project_id: &project.project_id,
        promoted_task_ids: &promoted_task_ids,
        promoted_run_ids: &promoted_run_ids,
        now_i64,
    };
    for dec in decisions {
        last_cursor = insert_promoted_decision(&tx, &ctx, dec).await?;
    }

    tx.commit().await?;

    let initial_cursor = CursorSeq(last_cursor);
    Ok(ServerPromotionAck {
        workspace_id: workspace_id.clone(),
        project_id: ProjectId::from(project.project_id.0),
        initial_cursor,
        ownership_epoch: epoch,
        acknowledged_at: now,
    })
}

async fn insert_promoted_project(
    tx: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    project: &TeamProjectSpec,
    epoch: u64,
    now_i64: i64,
) -> Result<u64, ServerError> {
    let budget_json = serde_json::to_string(&project.budget_policy)?;
    tx.execute(
        concat!(
            "INSERT INTO team_projects ",
            "(project_id, workspace_id, name, default_branch, max_concurrent_runs, ",
            "budget_policy_json, policy_digest, ownership_epoch, revision, created_at, updated_at) ",
            "VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        ),
        &[
            &project.project_id.as_ref(),
            &workspace_id.as_ref(),
            &project.name,
            &project.default_branch,
            &ServerError::to_pg_i32(project.max_concurrent_runs)?,
            &budget_json,
            &project.policy_digest.as_ref(),
            &ServerError::to_pg_i64(epoch)?,
            &ServerError::to_pg_i64(project.revision.0)?,
            &ServerError::to_pg_i64(project.created_at.0)?,
            &now_i64,
        ],
    )
    .await?;

    let event = WorkspaceSyncEvent::ProjectPromoted {
        project: TeamProjectSpec {
            ownership_epoch: epoch,
            ..project.clone()
        },
    };
    emit_workspace_event(
        tx,
        WorkspaceEventRecord {
            workspace_id,
            project_id: Some(project.project_id.as_ref()),
            aggregate_type: "project",
            aggregate_id: project.project_id.as_ref(),
            event_type: "project_promoted",
            event: &event,
            actor_user_id: actor.user_id.as_ref(),
            created_at: now_i64,
        },
    )
    .await
}

async fn insert_promoted_task(
    tx: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    project_id: &ProjectId,
    task: Task,
    now_i64: i64,
) -> Result<u64, ServerError> {
    if &task.project_id != project_id {
        return Err(ServerError::BadRequest(format!(
            "task {} does not belong to promoted project {project_id}",
            task.id
        )));
    }
    let budget_override_json = task
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
            "VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)",
        ),
        &[
            &task.id.as_ref(),
            &workspace_id.as_ref(),
            &project_id.as_ref(),
            &task.title,
            &task.description,
            &task_kind_to_str(task.kind),
            &task_stage_to_str(task.stage),
            &workflow_stage_to_str(task.workflow_stage),
            &task.plan_text,
            &ServerError::to_pg_i32(task.plan_version)?,
            &task.base_ref,
            &task.head_ref,
            &task.pr_url,
            &task.active_session_id.as_ref().map(|s| s.0.as_str()),
            &task.active_run_id.as_ref().map(|r| r.0.as_str()),
            &task.latest_run_id.as_ref().map(|r| r.0.as_str()),
            &budget_override_json,
            &ServerError::to_pg_i64(task.revision.0)?,
            &ServerError::to_pg_i64(task.created_at.0)?,
            &ServerError::to_pg_i64(task.updated_at.0)?,
        ],
    )
    .await?;

    let task_id_str = task.id.0.clone();
    let event = WorkspaceSyncEvent::TaskUpserted { task };
    emit_workspace_event(
        tx,
        WorkspaceEventRecord {
            workspace_id,
            project_id: Some(project_id.as_ref()),
            aggregate_type: "task",
            aggregate_id: &task_id_str,
            event_type: "task_upserted",
            event: &event,
            actor_user_id: actor.user_id.as_ref(),
            created_at: now_i64,
        },
    )
    .await
}

async fn insert_promoted_run(
    tx: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    actor: &AuthenticatedUser,
    project_id: &ProjectId,
    mut run: Run,
    promoted_task_ids: &HashSet<runlark_domain::TaskId>,
    now_i64: i64,
) -> Result<u64, ServerError> {
    if run.project_id.as_ref() != Some(project_id) {
        return Err(ServerError::BadRequest(format!(
            "promoted run {} does not belong to project {project_id}",
            run.id
        )));
    }
    let task_id = run.task_id.clone().ok_or_else(|| {
        ServerError::BadRequest(format!("promoted run {} must have task_id", run.id))
    })?;
    if !promoted_task_ids.contains(&task_id) {
        return Err(ServerError::BadRequest(format!(
            "promoted run {} references unknown task {task_id}",
            run.id
        )));
    }

    let wf_digest = run.workflow_digest.as_ref().map(ToString::to_string);
    let run_budget = serde_json::to_string(&run.budget_policy)?;
    let run_cost = serde_json::to_string(&run.cost_summary)?;
    tx.execute(
        concat!(
            "INSERT INTO team_runs ",
            "(run_id, workspace_id, project_id, task_id, session_id, status, workflow_stage, ",
            "task_kind, workflow_digest, base_sha, head_sha, plan_version, policy_digest, ",
            "budget_json, cost_summary_json, repair_attempts, pr_url, assigned_runner_id, ",
            "lease_epoch, runner_unreachable, revision, created_at, updated_at) ",
            "VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, NULL, 0, FALSE, $18, $19, $20)",
        ),
        &[
            &run.id.as_ref(),
            &workspace_id.as_ref(),
            &project_id.as_ref(),
            &task_id.as_ref(),
            &run.session_id.as_ref(),
            &run_status_to_str(run.status),
            &workflow_stage_to_str(run.workflow_stage),
            &task_kind_to_str(run.task_kind),
            &wf_digest,
            &run.base_sha,
            &run.head_sha,
            &ServerError::to_pg_i32(run.plan_version)?,
            &run.policy_digest.as_ref().map(|p| p.0.as_str()),
            &run_budget,
            &run_cost,
            &ServerError::to_pg_i32(run.repair_attempts)?,
            &run.pr_url,
            &ServerError::to_pg_i64(run.revision.0)?,
            &ServerError::to_pg_i64(run.created_at.0)?,
            &ServerError::to_pg_i64(run.updated_at.0)?,
        ],
    )
    .await?;

    // Strip local checkout worktree path from emitted sync event, matching team_runs persistence.
    run.worktree_path.clear();
    let run_id_str = run.id.0.clone();
    let event = WorkspaceSyncEvent::RunUpserted {
        run,
        ownership: RunOwnership::default(),
    };
    emit_workspace_event(
        tx,
        WorkspaceEventRecord {
            workspace_id,
            project_id: Some(project_id.as_ref()),
            aggregate_type: "run",
            aggregate_id: &run_id_str,
            event_type: "run_upserted",
            event: &event,
            actor_user_id: actor.user_id.as_ref(),
            created_at: now_i64,
        },
    )
    .await
}

async fn insert_promoted_decision(
    tx: &Transaction<'_>,
    ctx: &PromotionTxContext<'_>,
    dec: Decision,
) -> Result<u64, ServerError> {
    if !ctx.promoted_task_ids.contains(&dec.task_id) || !ctx.promoted_run_ids.contains(&dec.run_id)
    {
        return Err(ServerError::BadRequest(format!(
            "promoted decision {} must reference a promoted task and run",
            dec.id
        )));
    }
    tx.execute(
        concat!(
            "INSERT INTO team_decisions ",
            "(decision_id, workspace_id, task_id, run_id, kind, actor, plan_version, ",
            "head_sha, base_sha, policy_digest, evidence_digest, comment, created_at) ",
            "VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        ),
        &[
            &dec.id.as_ref(),
            &ctx.workspace_id.as_ref(),
            &dec.task_id.as_ref(),
            &dec.run_id.as_ref(),
            &decision_kind_to_str(dec.kind),
            &dec.actor,
            &ServerError::to_pg_i32(dec.plan_version)?,
            &dec.head_sha,
            &dec.base_sha,
            &dec.policy_digest.as_ref(),
            &dec.evidence_digest.as_str(),
            &dec.comment,
            &ServerError::to_pg_i64(dec.created_at.0)?,
        ],
    )
    .await?;

    let dec_id_str = dec.id.0.clone();
    let event = WorkspaceSyncEvent::DecisionRecorded { decision: dec };
    emit_workspace_event(
        tx,
        WorkspaceEventRecord {
            workspace_id: ctx.workspace_id,
            project_id: Some(ctx.project_id.as_ref()),
            aggregate_type: "decision",
            aggregate_id: &dec_id_str,
            event_type: "decision_recorded",
            event: &event,
            actor_user_id: ctx.actor.user_id.as_ref(),
            created_at: ctx.now_i64,
        },
    )
    .await
}
