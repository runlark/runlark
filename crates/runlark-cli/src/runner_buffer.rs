use crate::team_sync::TeamServerError;
use crate::worktree::WorktreeManager;
use runlark_domain::{
    EventSeq, LeaseEpoch, ProjectOwnershipMode, Run, RunId, RunOwnership, RunStatus,
    RunnerEventInput, RunnerId, RunnerRegistration, RunnerRunReport, Timestamp, WorkflowStage,
    WorkspaceId,
};
use runlark_local::{
    BufferedRunnerReport, ProjectStore, RunnerBufferStore, SessionStore, StorageError, SyncStore,
    TeamAuthStore, open_sqlite, with_immediate_tx,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Server response returned by `POST /runs/{run_id}/assign` and `POST /runs/{run_id}/runner-state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunOwnershipResponse {
    pub run: Run,
    pub ownership: RunOwnership,
}

/// Input payload for [`crate::runner_agent::RunnerAgentWorker::report_or_buffer_run_state`].
#[derive(Debug, Clone)]
pub struct RunnerStateInput<'a> {
    pub workspace_id: &'a WorkspaceId,
    pub run_id: &'a RunId,
    pub runner_id: &'a RunnerId,
    pub lease_epoch: LeaseEpoch,
    pub status: Option<RunStatus>,
    pub workflow_stage: Option<WorkflowStage>,
    pub head_sha: Option<String>,
    pub base_sha: Option<String>,
    pub event_type: &'a str,
    pub payload_json: &'a str,
}

/// Summary counters returned by a single [`crate::runner_agent::RunnerAgentWorker::tick_once`] pass.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerTickSummary {
    pub heartbeats_sent: usize,
    pub buffered_events_flushed: usize,
    pub assigned_runs_prepared: usize,
    pub fencing_conflicts: usize,
}

pub(crate) fn load_assigned_executable_runs(
    db_path: &Path,
    workspace_id: &WorkspaceId,
    runner_id: &RunnerId,
) -> Result<Vec<(Run, RunOwnership)>, StorageError> {
    let conn = open_sqlite(db_path)?;
    let mut owned = Vec::new();
    for binding in SyncStore::list_project_bindings(&conn)? {
        let matches_ws = matches!(
            &binding.ownership_mode,
            ProjectOwnershipMode::TeamManaged {
                workspace_id: wid, ..
            } if wid == workspace_id
        );
        if !matches_ws {
            continue;
        }
        let ownerships = SyncStore::list_run_ownerships_for_project(&conn, &binding.project_id)?;
        for run in ProjectStore::list_runs_for_project(&conn, &binding.project_id)? {
            if run.status.is_terminal() {
                continue;
            }
            if let Some(own) = ownerships.get(&run.id)
                && own.assigned_runner_id.as_ref() == Some(runner_id)
                && own.lease_epoch.0 > 0
                && !own.runner_unreachable
            {
                owned.push((run, own.clone()));
            }
        }
    }
    Ok(owned)
}

#[derive(Default)]
pub(crate) struct RunFlushOutcome {
    pub conflicts: usize,
    pub flushed_seqs: BTreeSet<EventSeq>,
    pub conflicted_seqs: BTreeSet<EventSeq>,
}

pub(crate) struct ReportChunk {
    pub report: RunnerRunReport,
    pub min_seq: EventSeq,
    pub max_seq: EventSeq,
}

pub(crate) fn is_fencing_conflict_code(code: &str) -> bool {
    matches!(code, "runner_fencing_mismatch" | "runner_unreachable")
}

pub(crate) fn persist_runner_registration(
    db_path: &Path,
    server_url: &str,
    reg: &RunnerRegistration,
    update_auth_identity: bool,
    now: Timestamp,
) -> Result<(), StorageError> {
    let mut conn = open_sqlite(db_path)?;
    with_immediate_tx::<_, StorageError, _>(&mut conn, |tx| {
        SyncStore::ensure_workspace_placeholder(tx, server_url, &reg.workspace_id, now)?;
        SyncStore::upsert_runner(tx, reg)?;
        if update_auth_identity {
            TeamAuthStore::set_runner_identity(
                tx,
                server_url,
                &reg.owner_user_id,
                &reg.workspace_id,
                &reg.id,
                &reg.name,
                now,
            )?;
        }
        Ok(())
    })
}

pub(crate) fn commit_flushed_chunk(
    db_path: &Path,
    run_id: &RunId,
    server_run: &Run,
    chunk: &ReportChunk,
    out: &mut RunFlushOutcome,
) -> Result<(), StorageError> {
    let mut conn = open_sqlite(db_path)?;
    with_immediate_tx::<_, StorageError, _>(&mut conn, |tx| {
        let should_save = SessionStore::get_run(tx, run_id)?
            .is_none_or(|local| server_run.revision >= local.revision);
        if should_save {
            SessionStore::save_run(tx, server_run)?;
        }
        RunnerBufferStore::mark_flushed_range(tx, run_id, chunk.min_seq, chunk.max_seq)?;
        Ok(())
    })?;
    out.flushed_seqs
        .extend(chunk.report.events.iter().map(|e| e.seq));
    Ok(())
}

pub(crate) fn mark_conflicted_chunk(
    db_path: &Path,
    run_id: &RunId,
    chunk: &ReportChunk,
    out: &mut RunFlushOutcome,
) -> Result<(), StorageError> {
    let conn = open_sqlite(db_path)?;
    RunnerBufferStore::mark_flushed_range(&conn, run_id, chunk.min_seq, chunk.max_seq)?;
    out.conflicted_seqs
        .extend(chunk.report.events.iter().map(|e| e.seq));
    Ok(())
}

pub(crate) async fn prepare_run_worktrees(
    db_path: &Path,
    assigned: &[(Run, RunOwnership)],
    now_ms: u64,
) -> Result<usize, TeamServerError> {
    let mut prepared = 0_usize;
    for (run, _) in assigned {
        if !run.worktree_path.trim().is_empty() {
            continue;
        }
        let Some(ref pid) = run.project_id else {
            continue;
        };
        let repo_root = {
            let conn = open_sqlite(db_path)?;
            ProjectStore::get_project(&conn, pid)?.map(|p| p.repo_root)
        };
        let Some(root) = repo_root.filter(|r| !r.trim().is_empty()) else {
            continue;
        };
        let root_path = PathBuf::from(&root);
        if !root_path.join(".git").exists() {
            continue;
        }
        let rid = run.id.clone();
        let wt =
            tokio::task::spawn_blocking(move || WorktreeManager::ensure_worktree(&root_path, &rid))
                .await
                .map_err(|e| TeamServerError::Storage(StorageError::InvalidState(e.to_string())))?;
        if let Ok(resolution) = wt {
            let conn = open_sqlite(db_path)?;
            SessionStore::update_run_worktree(
                &conn,
                &run.id,
                &resolution.worktree_path.to_string_lossy(),
                Timestamp(now_ms),
            )?;
            prepared += 1;
        }
    }
    Ok(prepared)
}

pub(crate) fn group_contiguous_reports(entries: &[BufferedRunnerReport]) -> Vec<ReportChunk> {
    let mut chunks: Vec<ReportChunk> = Vec::new();
    for row in entries {
        let ev = RunnerEventInput {
            seq: row.seq,
            event_type: row.event_type.clone(),
            payload_json: row.payload_json.clone(),
            created_at: row.created_at,
        };
        let same_header = chunks.last().is_some_and(|c| {
            c.report.runner_id == row.runner_id
                && c.report.lease_epoch == row.lease_epoch
                && c.report.status == row.status
                && c.report.workflow_stage == row.workflow_stage
                && c.report.head_sha == row.head_sha
                && c.report.base_sha == row.base_sha
        });
        if same_header && let Some(last) = chunks.last_mut() {
            last.report.events.push(ev);
            last.max_seq = row.seq;
        } else {
            chunks.push(ReportChunk {
                report: RunnerRunReport {
                    runner_id: row.runner_id.clone(),
                    lease_epoch: row.lease_epoch,
                    status: row.status,
                    workflow_stage: row.workflow_stage,
                    head_sha: row.head_sha.clone(),
                    base_sha: row.base_sha.clone(),
                    events: vec![ev],
                },
                min_seq: row.seq,
                max_seq: row.seq,
            });
        }
    }
    chunks
}
