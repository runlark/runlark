use super::{ProjectSyncBinding, SyncStore};
use crate::cas_store::ArtifactCasStore;
use crate::connection::to_sql_i64;
use crate::enum_codec::{
    artifact_kind_to_str, decision_kind_to_str, task_kind_to_str, task_stage_to_str,
    workflow_stage_to_str,
};
use crate::error::StorageError;
use runlark_domain::{
    ArtifactRef, CursorSeq, Decision, ProjectOwnershipMode, RunId, RunStatus, RunnerRegistration,
    Task, TeamProjectSpec, TeamRunEntry, Timestamp, WorkspaceId, WorkspaceMember,
};
use rusqlite::{Connection, params};

impl SyncStore {
    pub(super) fn replace_members_in_tx(
        tx: &Connection,
        workspace_id: &WorkspaceId,
        members: &[WorkspaceMember],
    ) -> Result<(), StorageError> {
        tx.execute(
            "DELETE FROM team_members WHERE workspace_id = ?1",
            params![workspace_id.as_ref()],
        )?;
        for member in members {
            Self::upsert_member(tx, member)?;
        }
        Ok(())
    }

    pub(super) fn replace_runners_in_tx(
        tx: &Connection,
        workspace_id: &WorkspaceId,
        runners: &[RunnerRegistration],
    ) -> Result<(), StorageError> {
        tx.execute(
            "DELETE FROM team_runners WHERE workspace_id = ?1",
            params![workspace_id.as_ref()],
        )?;
        for runner in runners {
            Self::upsert_runner(tx, runner)?;
        }
        Ok(())
    }

    pub(super) fn upsert_synced_project_in_tx(
        tx: &Connection,
        server_url: &str,
        spec: &TeamProjectSpec,
        cursor_seq: CursorSeq,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let budget_json = serde_json::to_string(&spec.budget_policy)?;
        tx.execute(
            concat!(
                "INSERT INTO projects (",
                "project_id, name, repo_root, default_branch, max_concurrent_runs, ",
                "budget_policy_json, policy_digest, revision, created_at, updated_at) ",
                "VALUES (?1, ?2, '', ?3, ?4, ?5, ?6, ?7, ?8, ?9) ",
                "ON CONFLICT(project_id) DO UPDATE SET ",
                "name = excluded.name, default_branch = excluded.default_branch, ",
                "max_concurrent_runs = excluded.max_concurrent_runs, ",
                "budget_policy_json = excluded.budget_policy_json, ",
                "policy_digest = excluded.policy_digest, revision = excluded.revision, ",
                "updated_at = excluded.updated_at"
            ),
            params![
                spec.project_id.as_ref(),
                spec.name,
                spec.default_branch,
                i64::from(spec.max_concurrent_runs),
                budget_json,
                spec.policy_digest.as_ref(),
                to_sql_i64(spec.revision.0)?,
                to_sql_i64(spec.created_at.0)?,
                to_sql_i64(spec.updated_at.0)?,
            ],
        )?;
        let binding = ProjectSyncBinding {
            project_id: spec.project_id.clone(),
            ownership_mode: ProjectOwnershipMode::TeamManaged {
                workspace_id: spec.workspace_id.clone(),
                server_url: server_url.to_string(),
                ownership_epoch: spec.ownership_epoch,
            },
            last_cursor_seq: cursor_seq,
            updated_at: now,
        };
        Self::upsert_binding_in_tx(tx, &binding)
    }

    pub(super) fn upsert_task_in_tx(tx: &Connection, task: &Task) -> Result<(), StorageError> {
        let budget_json = task
            .budget_override
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        tx.execute(
            concat!(
                "INSERT INTO tasks (",
                "task_id, project_id, title, description, kind, stage, workflow_stage, ",
                "plan_text, plan_version, base_ref, head_ref, pr_url, active_session_id, ",
                "active_run_id, latest_run_id, budget_override_json, revision, created_at, updated_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19) ",
                "ON CONFLICT(task_id) DO UPDATE SET ",
                "title = excluded.title, description = excluded.description, kind = excluded.kind, ",
                "stage = excluded.stage, workflow_stage = excluded.workflow_stage, ",
                "plan_text = excluded.plan_text, plan_version = excluded.plan_version, ",
                "base_ref = excluded.base_ref, head_ref = excluded.head_ref, pr_url = excluded.pr_url, ",
                "active_session_id = excluded.active_session_id, active_run_id = excluded.active_run_id, ",
                "latest_run_id = excluded.latest_run_id, budget_override_json = excluded.budget_override_json, ",
                "revision = excluded.revision, updated_at = excluded.updated_at"
            ),
            params![
                task.id.as_ref(),
                task.project_id.as_ref(),
                task.title,
                task.description,
                task_kind_to_str(task.kind),
                task_stage_to_str(task.stage),
                workflow_stage_to_str(task.workflow_stage),
                task.plan_text,
                i64::from(task.plan_version),
                task.base_ref,
                task.head_ref,
                task.pr_url,
                task.active_session_id.as_ref().map(AsRef::as_ref),
                task.active_run_id.as_ref().map(AsRef::as_ref),
                task.latest_run_id.as_ref().map(AsRef::as_ref),
                budget_json,
                to_sql_i64(task.revision.0)?,
                to_sql_i64(task.created_at.0)?,
                to_sql_i64(task.updated_at.0)?,
            ],
        )?;
        Ok(())
    }

    pub(super) fn upsert_run_entry_in_tx(
        tx: &Connection,
        entry: &TeamRunEntry,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let run = &entry.run;
        let task_ref = run.task_id.as_ref().map_or("", AsRef::as_ref);
        tx.execute(
            concat!(
                "INSERT OR IGNORE INTO sessions (",
                "session_id, task_id, cwd, status, current_revision, active_run_id, created_at, updated_at) ",
                "VALUES (?1, ?2, '', 'active', 1, ?3, ?4, ?5)"
            ),
            params![
                run.session_id.as_ref(),
                task_ref,
                run.id.as_ref(),
                to_sql_i64(run.created_at.0)?,
                to_sql_i64(run.updated_at.0)?,
            ],
        )?;

        let budget_json = serde_json::to_string(&run.budget_policy)?;
        let cost_json = serde_json::to_string(&run.cost_summary)?;
        let status_str = match run.status {
            RunStatus::Queued => "queued",
            RunStatus::Running => "running",
            RunStatus::WaitingForApproval => "waiting_for_approval",
            RunStatus::CancelRequested => "cancel_requested",
            RunStatus::Stopped => "stopped",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
            RunStatus::Interrupted => "interrupted",
        };
        tx.execute(
            concat!(
                "INSERT INTO runs (",
                "run_id, session_id, status, worktree_path, revision, created_at, updated_at, ",
                "task_id, project_id, workflow_stage, task_kind, workflow_digest, base_sha, head_sha, ",
                "plan_version, policy_digest, budget_json, cost_summary_json, repair_attempts, pr_url) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20) ",
                "ON CONFLICT(run_id) DO UPDATE SET ",
                "status = excluded.status, ",
                "worktree_path = CASE WHEN excluded.worktree_path != '' THEN excluded.worktree_path ELSE runs.worktree_path END, ",
                "revision = excluded.revision, updated_at = excluded.updated_at, ",
                "task_id = COALESCE(excluded.task_id, runs.task_id), ",
                "project_id = COALESCE(excluded.project_id, runs.project_id), ",
                "workflow_stage = excluded.workflow_stage, task_kind = excluded.task_kind, ",
                "workflow_digest = COALESCE(excluded.workflow_digest, runs.workflow_digest), ",
                "base_sha = COALESCE(excluded.base_sha, runs.base_sha), ",
                "head_sha = COALESCE(excluded.head_sha, runs.head_sha), ",
                "plan_version = excluded.plan_version, ",
                "policy_digest = COALESCE(excluded.policy_digest, runs.policy_digest), ",
                "budget_json = excluded.budget_json, cost_summary_json = excluded.cost_summary_json, ",
                "repair_attempts = excluded.repair_attempts, ",
                "pr_url = COALESCE(excluded.pr_url, runs.pr_url)"
            ),
            params![
                run.id.as_ref(),
                run.session_id.as_ref(),
                status_str,
                run.worktree_path,
                to_sql_i64(run.revision.0)?,
                to_sql_i64(run.created_at.0)?,
                to_sql_i64(run.updated_at.0)?,
                run.task_id.as_ref().map(AsRef::as_ref),
                run.project_id.as_ref().map(AsRef::as_ref),
                workflow_stage_to_str(run.workflow_stage),
                task_kind_to_str(run.task_kind),
                run.workflow_digest.as_ref().map(AsRef::as_ref),
                run.base_sha,
                run.head_sha,
                i64::from(run.plan_version),
                run.policy_digest.as_ref().map(AsRef::as_ref),
                budget_json,
                cost_json,
                i64::from(run.repair_attempts),
                run.pr_url,
            ],
        )?;

        tx.execute(
            concat!(
                "INSERT INTO run_ownership (",
                "run_id, assigned_runner_id, lease_epoch, runner_unreachable, updated_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5) ",
                "ON CONFLICT(run_id) DO UPDATE SET ",
                "assigned_runner_id = excluded.assigned_runner_id, ",
                "lease_epoch = excluded.lease_epoch, ",
                "runner_unreachable = excluded.runner_unreachable, ",
                "updated_at = excluded.updated_at"
            ),
            params![
                run.id.as_ref(),
                entry
                    .ownership
                    .assigned_runner_id
                    .as_ref()
                    .map(AsRef::as_ref),
                to_sql_i64(entry.ownership.lease_epoch.0)?,
                i64::from(entry.ownership.runner_unreachable),
                to_sql_i64(now.0)?,
            ],
        )?;
        Ok(())
    }

    pub(super) fn insert_decision_in_tx(
        tx: &Connection,
        decision: &Decision,
    ) -> Result<(), StorageError> {
        tx.execute(
            concat!(
                "INSERT OR IGNORE INTO decisions (",
                "decision_id, task_id, run_id, kind, actor, plan_version, head_sha, base_sha, ",
                "policy_digest, evidence_digest, comment, created_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
            ),
            params![
                decision.id.as_ref(),
                decision.task_id.as_ref(),
                decision.run_id.as_ref(),
                decision_kind_to_str(decision.kind),
                decision.actor,
                i64::from(decision.plan_version),
                decision.head_sha,
                decision.base_sha,
                decision.policy_digest.as_ref(),
                decision.evidence_digest.as_str(),
                decision.comment,
                to_sql_i64(decision.created_at.0)?,
            ],
        )?;
        Ok(())
    }

    pub(super) fn upsert_artifact_link_in_tx(
        tx: &Connection,
        run_id: &RunId,
        artifact: &ArtifactRef,
        label: &str,
    ) -> Result<(), StorageError> {
        tx.execute(
            concat!(
                "INSERT OR IGNORE INTO artifacts (digest_sha256, kind, media_type, size_bytes, created_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5)"
            ),
            params![
                artifact.digest_sha256.as_str(),
                artifact_kind_to_str(artifact.kind),
                artifact.media_type,
                to_sql_i64(artifact.size_bytes)?,
                to_sql_i64(artifact.created_at.0)?,
            ],
        )?;
        ArtifactCasStore::link_to_run(
            tx,
            run_id,
            &artifact.digest_sha256,
            label,
            artifact.created_at,
        )
    }

    pub(super) fn upsert_binding_in_tx(
        tx: &Connection,
        binding: &ProjectSyncBinding,
    ) -> Result<(), StorageError> {
        let (mode_str, workspace_id, server_url, epoch) = match &binding.ownership_mode {
            ProjectOwnershipMode::SoloLocal => ("solo_local", None, None, 0_u64),
            ProjectOwnershipMode::TeamManaged {
                workspace_id,
                server_url,
                ownership_epoch,
            } => (
                "team_managed",
                Some(workspace_id.as_ref()),
                Some(server_url.as_str()),
                *ownership_epoch,
            ),
        };
        tx.execute(
            concat!(
                "INSERT INTO project_sync_bindings (",
                "project_id, ownership_mode, workspace_id, server_url, ownership_epoch, ",
                "last_cursor_seq, updated_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ",
                "ON CONFLICT(project_id) DO UPDATE SET ",
                "ownership_mode = excluded.ownership_mode, workspace_id = excluded.workspace_id, ",
                "server_url = excluded.server_url, ownership_epoch = excluded.ownership_epoch, ",
                "last_cursor_seq = excluded.last_cursor_seq, updated_at = excluded.updated_at"
            ),
            params![
                binding.project_id.as_ref(),
                mode_str,
                workspace_id,
                server_url,
                to_sql_i64(epoch)?,
                to_sql_i64(binding.last_cursor_seq.0)?,
                to_sql_i64(binding.updated_at.0)?,
            ],
        )?;
        Ok(())
    }

    /// Ensures a placeholder row exists in `team_workspaces` before upserting child rows
    /// (`team_runners`, `team_members`) prior to initial snapshot sync.
    ///
    /// # Errors
    /// Returns [`StorageError`] on SQL failure.
    pub fn ensure_workspace_placeholder(
        conn: &Connection,
        server_url: &str,
        workspace_id: &WorkspaceId,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        Self::ensure_workspace_placeholder_in_tx(
            conn,
            server_url.trim_end_matches('/'),
            workspace_id,
            now,
        )
    }

    pub(super) fn ensure_workspace_placeholder_in_tx(
        tx: &Connection,
        server_url: &str,
        workspace_id: &WorkspaceId,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let now_sql = to_sql_i64(now.0)?;
        tx.execute(
            concat!(
                "INSERT INTO team_workspaces (",
                "workspace_id, server_url, name, slug, created_by, revision, last_cursor_seq, created_at, updated_at) ",
                "VALUES (?1, ?2, ?1, ?1, '', 1, 0, ?3, ?3) ",
                "ON CONFLICT(workspace_id) DO UPDATE SET ",
                "server_url = excluded.server_url, ",
                "updated_at = excluded.updated_at"
            ),
            params![workspace_id.as_ref(), server_url, now_sql],
        )?;
        Ok(())
    }

    pub(super) fn set_workspace_cursor_in_tx(
        tx: &Connection,
        workspace_id: &WorkspaceId,
        cursor_seq: CursorSeq,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let cur_sql = to_sql_i64(cursor_seq.0)?;
        let now_sql = to_sql_i64(now.0)?;
        tx.execute(
            "UPDATE team_workspaces SET last_cursor_seq = ?2, updated_at = ?3 WHERE workspace_id = ?1",
            params![workspace_id.as_ref(), cur_sql, now_sql],
        )?;
        Self::update_workspace_bindings_cursor_in_tx(tx, workspace_id, cursor_seq, now)
    }

    pub(super) fn update_workspace_bindings_cursor_in_tx(
        tx: &Connection,
        workspace_id: &WorkspaceId,
        cursor_seq: CursorSeq,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        tx.execute(
            "UPDATE project_sync_bindings SET last_cursor_seq = ?2, updated_at = ?3 WHERE workspace_id = ?1",
            params![
                workspace_id.as_ref(),
                to_sql_i64(cursor_seq.0)?,
                to_sql_i64(now.0)?,
            ],
        )?;
        Ok(())
    }
}
