use super::{ProjectSyncBinding, PromotionBundle, SyncStore};
use crate::attempt_store::AttemptStore;
use crate::comment_store::CommentStore;
use crate::connection::with_immediate_tx;
use crate::decision_store::DecisionStore;
use crate::error::StorageError;
use crate::project_store::ProjectStore;
use crate::task_store::TaskStore;
use runlark_domain::{
    CursorSeq, ProjectId, ProjectOwnershipMode, ServerPromotionAck, TeamRunEntry, Timestamp,
    WorkspaceEventEnvelope, WorkspaceId, WorkspaceSnapshot, WorkspaceSyncEvent,
    validate_solo_to_team_promotion,
};
use rusqlite::Connection;

impl SyncStore {
    /// Atomically applies a full [`WorkspaceSnapshot`] and advances `last_cursor_seq` inside one
    /// `TransactionBehavior::Immediate` transaction, preserving local `repo_root` and `worktree_path`.
    ///
    /// # Errors
    /// Returns [`StorageError`] and rolls back the entire snapshot if any step fails.
    pub fn apply_snapshot_atomic(
        conn: &mut Connection,
        server_url: &str,
        snapshot: &WorkspaceSnapshot,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let clean_url = server_url.trim_end_matches('/');
        with_immediate_tx(conn, |tx| {
            let existing_cursor = Self::get_workspace_cursor(tx, &snapshot.workspace.id)?;
            if snapshot.cursor_seq.0 < existing_cursor.0 {
                return Ok(());
            }
            Self::upsert_workspace_metadata(
                tx,
                clean_url,
                &snapshot.workspace,
                snapshot.cursor_seq,
            )?;
            Self::replace_members_in_tx(tx, &snapshot.workspace.id, &snapshot.members)?;
            Self::replace_runners_in_tx(tx, &snapshot.workspace.id, &snapshot.runners)?;

            for spec in &snapshot.projects {
                Self::upsert_synced_project_in_tx(tx, clean_url, spec, snapshot.cursor_seq, now)?;
            }
            for task in &snapshot.tasks {
                Self::upsert_task_in_tx(tx, task)?;
            }
            for entry in &snapshot.runs {
                Self::upsert_run_entry_in_tx(tx, entry, now)?;
            }
            for attempt in &snapshot.attempts {
                AttemptStore::save_attempt(tx, attempt)?;
            }
            for decision in &snapshot.decisions {
                Self::insert_decision_in_tx(tx, decision)?;
            }
            for comment in &snapshot.comments {
                CommentStore::upsert_comment(tx, comment)?;
            }
            for entry in &snapshot.artifacts {
                Self::upsert_artifact_link_in_tx(tx, &entry.run_id, &entry.artifact, &entry.label)?;
            }
            Self::update_workspace_bindings_cursor_in_tx(
                tx,
                &snapshot.workspace.id,
                snapshot.cursor_seq,
                now,
            )?;
            Ok(())
        })
    }

    /// Atomically applies an ordered batch of [`WorkspaceEventEnvelope`] records inside one
    /// `TransactionBehavior::Immediate` transaction, validating contiguous `cursor_seq` progression
    /// and rolling back on any gap (`StorageError::CursorGap`) or error.
    ///
    /// # Errors
    /// Returns [`StorageError::CursorGap`] if `events` skip a sequence number, [`StorageError::InvalidState`]
    /// on workspace mismatch, or [`StorageError`] on SQL/conversion failure.
    pub fn apply_events_batch_atomic(
        conn: &mut Connection,
        workspace_id: &WorkspaceId,
        server_url: &str,
        events: &[WorkspaceEventEnvelope],
        now: Timestamp,
    ) -> Result<CursorSeq, StorageError> {
        let clean_url = server_url.trim_end_matches('/');
        with_immediate_tx(conn, |tx| {
            let mut cursor = Self::get_workspace_cursor(tx, workspace_id)?;
            if events.is_empty() {
                return Ok(cursor);
            }

            for env in events {
                if &env.workspace_id != workspace_id {
                    return Err(StorageError::InvalidState(format!(
                        "event cursor {} belongs to workspace {}, expected {workspace_id}",
                        env.cursor_seq, env.workspace_id
                    )));
                }
                if env.cursor_seq.0 <= cursor.0 {
                    continue;
                }
                let expected = CursorSeq(cursor.0.saturating_add(1));
                if env.cursor_seq != expected {
                    return Err(StorageError::CursorGap {
                        expected,
                        actual: env.cursor_seq,
                    });
                }
                Self::apply_single_sync_event_in_tx(tx, clean_url, env, now)?;
                cursor = env.cursor_seq;
            }

            Self::set_workspace_cursor_in_tx(tx, workspace_id, cursor, now)?;
            Ok(cursor)
        })
    }

    /// Validates that a local `SoloLocal` project exists and has zero non-terminal runs,
    /// returning a [`PromotionBundle`] ready for sanitized server promotion.
    ///
    /// # Errors
    /// Returns [`StorageError::ProjectNotFound`] if missing, [`StorageError::InvalidState`] if
    /// already `TeamManaged`, or [`StorageError::Transition`] if any run is non-terminal.
    pub fn validate_project_promotion_eligible(
        conn: &Connection,
        project_id: &ProjectId,
    ) -> Result<PromotionBundle, StorageError> {
        let project = ProjectStore::get_project(conn, project_id)?
            .ok_or_else(|| StorageError::ProjectNotFound(project_id.clone()))?;
        if let ProjectOwnershipMode::TeamManaged { workspace_id, .. } =
            Self::get_ownership_mode(conn, project_id)?
        {
            return Err(StorageError::InvalidState(format!(
                "project {project_id} is already team-managed in workspace {workspace_id}"
            )));
        }

        let tasks = TaskStore::list_tasks(conn, Some(project_id), None)?;
        let runs = ProjectStore::list_runs_for_project(conn, project_id)?;
        validate_solo_to_team_promotion(&runs)?;

        let mut decisions = Vec::new();
        for task in &tasks {
            decisions.extend(DecisionStore::list_decisions_for_task(conn, &task.id)?);
        }
        Ok(PromotionBundle {
            project,
            tasks,
            runs,
            decisions,
        })
    }

    /// Atomically promotes a local project to `TeamManaged` inside `with_immediate_tx` after
    /// verifying zero non-terminal runs and persisting `ownership_epoch` + `last_cursor_seq`.
    ///
    /// # Errors
    /// Returns [`StorageError`] if project is missing, has active runs, or is already promoted.
    pub fn promote_project_to_team_atomic(
        conn: &mut Connection,
        project_id: &ProjectId,
        server_url: &str,
        ack: &ServerPromotionAck,
        now: Timestamp,
    ) -> Result<ProjectSyncBinding, StorageError> {
        if &ack.project_id != project_id {
            return Err(StorageError::InvalidState(format!(
                "ServerPromotionAck project_id {} does not match {project_id}",
                ack.project_id
            )));
        }
        let clean_url = server_url.trim_end_matches('/');
        with_immediate_tx(conn, |tx| {
            let project = ProjectStore::get_project(tx, project_id)?
                .ok_or_else(|| StorageError::ProjectNotFound(project_id.clone()))?;
            if let ProjectOwnershipMode::TeamManaged { workspace_id, .. } =
                Self::get_ownership_mode(tx, project_id)?
            {
                return Err(StorageError::InvalidState(format!(
                    "project {project_id} is already team-managed in workspace {workspace_id}"
                )));
            }
            let runs = ProjectStore::list_runs_for_project(tx, project_id)?;
            validate_solo_to_team_promotion(&runs)?;

            Self::ensure_workspace_placeholder_in_tx(tx, clean_url, &ack.workspace_id, now)?;

            let binding = ProjectSyncBinding {
                project_id: project.id,
                ownership_mode: ProjectOwnershipMode::TeamManaged {
                    workspace_id: ack.workspace_id.clone(),
                    server_url: clean_url.to_string(),
                    ownership_epoch: ack.ownership_epoch,
                },
                last_cursor_seq: ack.initial_cursor,
                updated_at: now,
            };
            Self::upsert_binding_in_tx(tx, &binding)?;
            Ok(binding)
        })
    }

    fn apply_single_sync_event_in_tx(
        tx: &Connection,
        server_url: &str,
        env: &WorkspaceEventEnvelope,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        match &env.event {
            WorkspaceSyncEvent::ProjectPromoted { project } => {
                Self::upsert_synced_project_in_tx(tx, server_url, project, env.cursor_seq, now)?;
            }
            WorkspaceSyncEvent::TaskUpserted { task } => {
                Self::upsert_task_in_tx(tx, task)?;
            }
            WorkspaceSyncEvent::RunUpserted { run, ownership } => {
                Self::upsert_run_entry_in_tx(
                    tx,
                    &TeamRunEntry {
                        run: run.clone(),
                        ownership: ownership.clone(),
                    },
                    now,
                )?;
            }
            WorkspaceSyncEvent::CommentAppended { comment } => {
                CommentStore::upsert_comment(tx, comment)?;
            }
            WorkspaceSyncEvent::AttemptRecorded { attempt } => {
                AttemptStore::save_attempt(tx, attempt)?;
            }
            WorkspaceSyncEvent::DecisionRecorded { decision } => {
                Self::insert_decision_in_tx(tx, decision)?;
            }
            WorkspaceSyncEvent::ArtifactLinked {
                run_id,
                artifact,
                label,
            } => {
                Self::upsert_artifact_link_in_tx(tx, run_id, artifact, label)?;
            }
            WorkspaceSyncEvent::RunnerUpdated { runner } => {
                Self::upsert_runner(tx, runner)?;
            }
            WorkspaceSyncEvent::MemberUpdated { member } => {
                Self::upsert_member(tx, member)?;
            }
        }
        Ok(())
    }
}
