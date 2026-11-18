use crate::team_sync::TeamServerError;
use crate::worktree::WorktreeManager;
use runlark_domain::{
    EventSeq, Run, RunId, RunOwnership, RunnerEventInput, RunnerRunReport, Timestamp,
};
use runlark_local::{
    BufferedRunnerReport, ProjectStore, RunnerBufferStore, SessionStore, StorageError, open_sqlite,
    with_immediate_tx,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

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

pub(crate) fn commit_flushed_chunk(
    db_path: &Path,
    run_id: &RunId,
    server_run: &Run,
    chunk: &ReportChunk,
    out: &mut RunFlushOutcome,
) -> Result<(), StorageError> {
    let mut conn = open_sqlite(db_path)?;
    with_immediate_tx::<_, StorageError, _>(&mut conn, |tx| {
        SessionStore::save_run(tx, server_run)?;
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
