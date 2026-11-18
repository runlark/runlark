use crate::runner_buffer::{
    RunFlushOutcome, commit_flushed_chunk, group_contiguous_reports, mark_conflicted_chunk,
    prepare_run_worktrees,
};
use crate::state::AppState;
use crate::team_sync::{TeamServerError, parse_server_json_response};
use reqwest::Client;
use runlark_domain::{
    EventSeq, LeaseEpoch, ProjectOwnershipMode, Run, RunId, RunOwnership, RunStatus,
    RunnerEventInput, RunnerId, RunnerRegistration, RunnerRunReport, Timestamp, WorkflowStage,
    WorkspaceId, validate_runner_fencing,
};
use runlark_local::{
    ProjectStore, RunnerBufferStore, StorageError, SyncStore, TeamAuthStore, open_sqlite,
    with_immediate_tx,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Notify, watch};

enum ChunkPostOutcome {
    Committed(Box<RunOwnershipResponse>),
    PermanentConflict,
    RunnerNotOnlineDeferred,
}

/// Server response returned by `POST /runs/{run_id}/assign` and `POST /runs/{run_id}/runner-state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunOwnershipResponse {
    pub run: Run,
    pub ownership: RunOwnership,
}

/// Input payload for [`RunnerAgentWorker::report_or_buffer_run_state`].
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

/// Summary counters returned by a single [`RunnerAgentWorker::tick_once`] pass.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerTickSummary {
    pub heartbeats_sent: usize,
    pub buffered_events_flushed: usize,
    pub assigned_runs_prepared: usize,
    pub fencing_conflicts: usize,
}

/// Background and on-demand worker managing runner registration, heartbeats, strict
/// `(runner_id, lease_epoch)` fenced run execution, and offline run-event buffering/reconciliation.
#[derive(Clone)]
pub struct RunnerAgentWorker {
    state: AppState,
    http: Client,
    poll_interval: Duration,
}

impl RunnerAgentWorker {
    /// Creates a [`RunnerAgentWorker`] bound to `state`.
    ///
    /// # Panics
    /// Panics only if building the underlying `reqwest::Client` fails.
    #[must_use]
    pub fn new(state: AppState) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client must build");
        Self {
            state,
            http,
            poll_interval: Duration::from_secs(2),
        }
    }

    /// Sets a custom background heartbeat/flush poll interval.
    #[must_use]
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Runs the background heartbeat, buffer flush, and assigned-run preparation loop.
    pub async fn run_loop(
        self: Arc<Self>,
        notify: Arc<Notify>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        loop {
            if *shutdown.borrow() {
                break;
            }
            if let Err(err) = self.tick_once().await {
                tracing::debug!("Runner agent tick deferred: {err}");
            }
            tokio::select! {
                () = notify.notified() => {},
                () = tokio::time::sleep(self.poll_interval) => {},
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
            }
        }
    }

    /// Executes one heartbeat + buffered-event flush + assigned-run worktree preparation pass.
    ///
    /// # Errors
    /// Returns [`TeamServerError`] if reading local `SQLite` state fails.
    pub async fn tick_once(&self) -> Result<RunnerTickSummary, TeamServerError> {
        let sessions = {
            let conn = open_sqlite(&self.state.db_path)?;
            TeamAuthStore::list_sessions(&conn)?
        };

        let mut summary = RunnerTickSummary::default();
        for sess in sessions {
            let (Some(wid), Some(rid)) =
                (sess.default_workspace_id.as_ref(), sess.runner_id.as_ref())
            else {
                continue;
            };
            if self
                .send_heartbeat(&sess.server_url, &sess.bearer_token, wid, rid)
                .await
                .is_ok()
            {
                summary.heartbeats_sent += 1;
            }

            match self
                .flush_buffered_events(&sess.server_url, &sess.bearer_token, wid)
                .await
            {
                Ok((flushed, conflicts)) => {
                    summary.buffered_events_flushed += flushed;
                    summary.fencing_conflicts += conflicts;
                }
                Err(TeamServerError::Offline(_)) => {}
                Err(err) => return Err(err),
            }

            let assigned = self.list_assigned_executable_runs(wid, rid)?;
            summary.assigned_runs_prepared += prepare_run_worktrees(
                &self.state.db_path,
                &assigned,
                AppState::current_timestamp_ms(),
            )
            .await?;
        }
        Ok(summary)
    }

    /// Registers this daemon as a runner in `workspace_id` and persists its `(runner_id, name)`.
    ///
    /// # Errors
    /// Returns [`TeamServerError`] on HTTP/auth failure or `SQLite` error.
    pub async fn register_runner(
        &self,
        server_url: &str,
        bearer_token: &str,
        workspace_id: &WorkspaceId,
        runner_id: Option<RunnerId>,
        name: &str,
    ) -> Result<RunnerRegistration, TeamServerError> {
        let url = format!(
            "{}/api/v1/workspaces/{}/runners/register",
            server_url.trim_end_matches('/'),
            workspace_id.as_ref()
        );
        let body = json!({
            "runner_id": runner_id.map(|r| r.0),
            "name": name,
        });
        let resp = self
            .http
            .post(&url)
            .bearer_auth(bearer_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| TeamServerError::Offline(e.to_string()))?;
        let reg: RunnerRegistration = parse_server_json_response(resp).await?;

        let now = Timestamp(AppState::current_timestamp_ms());
        let conn = open_sqlite(&self.state.db_path)?;
        SyncStore::upsert_runner(&conn, &reg)?;
        TeamAuthStore::set_runner_identity(&conn, server_url, &reg.id, &reg.name, now)?;
        Ok(reg)
    }

    /// Sends a heartbeat for `runner_id` in `workspace_id` and caches the refreshed registration.
    ///
    /// # Errors
    /// Returns [`TeamServerError`] on HTTP/auth/offline failure.
    pub async fn send_heartbeat(
        &self,
        server_url: &str,
        bearer_token: &str,
        workspace_id: &WorkspaceId,
        runner_id: &RunnerId,
    ) -> Result<RunnerRegistration, TeamServerError> {
        let url = format!(
            "{}/api/v1/workspaces/{}/runners/{}/heartbeat",
            server_url.trim_end_matches('/'),
            workspace_id.as_ref(),
            runner_id.as_ref()
        );
        let resp = self
            .http
            .post(&url)
            .bearer_auth(bearer_token)
            .send()
            .await
            .map_err(|e| TeamServerError::Offline(e.to_string()))?;
        let reg: RunnerRegistration = parse_server_json_response(resp).await?;
        let conn = open_sqlite(&self.state.db_path)?;
        SyncStore::upsert_runner(&conn, &reg)?;
        Ok(reg)
    }

    /// Records a fenced runner state update/event locally first in `runner_buffered_events`
    /// (verifying `(runner_id, lease_epoch)` ownership) and flushes to the server when online.
    ///
    /// Returns `Ok(true)` if immediately acknowledged by the server, or `Ok(false)` if buffered
    /// offline for ordered reconciliation on reconnect.
    ///
    /// # Errors
    /// Returns [`TeamServerError::Conflict`] if local ownership does not match `(runner_id, lease_epoch)`,
    /// or [`TeamServerError`] on server rejection or `SQLite` error.
    pub async fn report_or_buffer_run_state(
        &self,
        input: RunnerStateInput<'_>,
    ) -> Result<bool, TeamServerError> {
        let (server_url, bearer_token, seq) = {
            let mut conn = open_sqlite(&self.state.db_path)?;
            let now = Timestamp(AppState::current_timestamp_ms());
            with_immediate_tx::<_, TeamServerError, _>(&mut conn, |tx| {
                let ownership = SyncStore::get_run_ownership(tx, input.run_id)?.unwrap_or_default();
                validate_runner_fencing(
                    input.run_id,
                    &ownership,
                    input.runner_id,
                    input.lease_epoch,
                )
                .map_err(|e| TeamServerError::Conflict {
                    code: "runner_fencing_mismatch".to_string(),
                    message: e.to_string(),
                    current_revision: None,
                })?;
                if ownership.runner_unreachable {
                    return Err(TeamServerError::Conflict {
                        code: "runner_unreachable".to_string(),
                        message: format!("run {} owner is marked unreachable", input.run_id),
                        current_revision: None,
                    });
                }

                let auth = TeamAuthStore::resolve_for_workspace(tx, Some(input.workspace_id))?
                    .ok_or_else(|| {
                        TeamServerError::Unauthorized("no team auth session configured".to_string())
                    })?;
                let draft_report = RunnerRunReport {
                    runner_id: input.runner_id.clone(),
                    lease_epoch: input.lease_epoch,
                    status: input.status,
                    workflow_stage: input.workflow_stage,
                    head_sha: input.head_sha,
                    base_sha: input.base_sha,
                    events: vec![RunnerEventInput {
                        seq: EventSeq(0),
                        event_type: input.event_type.to_string(),
                        payload_json: input.payload_json.to_string(),
                        created_at: now,
                    }],
                };
                let seq = RunnerBufferStore::allocate_and_buffer_in_tx(
                    tx,
                    input.workspace_id,
                    input.run_id,
                    &draft_report,
                )?;
                Ok((auth.server_url, auth.bearer_token, seq))
            })?
        };

        match self
            .flush_run_buffered_events(&server_url, &bearer_token, input.workspace_id, input.run_id)
            .await
        {
            Ok(outcome) => {
                if outcome.conflicted_seqs.contains(&seq) {
                    return Err(TeamServerError::Conflict {
                        code: "runner_fencing_mismatch".to_string(),
                        message: "server rejected buffered runner state with 409".to_string(),
                        current_revision: None,
                    });
                }
                Ok(outcome.flushed_seqs.contains(&seq))
            }
            Err(TeamServerError::Offline(_)) => Ok(false),
            Err(err) => Err(err),
        }
    }

    /// Flushes buffered runner events for `workspace_id` in strict `(run_id, seq)` order.
    ///
    /// # Errors
    /// Returns [`TeamServerError::Offline`] if the server cannot be reached (preserving buffer).
    pub async fn flush_buffered_events(
        &self,
        server_url: &str,
        bearer_token: &str,
        workspace_id: &WorkspaceId,
    ) -> Result<(usize, usize), TeamServerError> {
        let run_ids: BTreeSet<RunId> = {
            let conn = open_sqlite(&self.state.db_path)?;
            RunnerBufferStore::list_unflushed(&conn, Some(workspace_id))?
                .into_iter()
                .map(|r| r.run_id)
                .collect()
        };
        let mut flushed_count = 0_usize;
        let mut conflict_count = 0_usize;
        for run_id in run_ids {
            let outcome = self
                .flush_run_buffered_events(server_url, bearer_token, workspace_id, &run_id)
                .await?;
            flushed_count += outcome.flushed_seqs.len();
            conflict_count += outcome.conflicts;
        }
        Ok((flushed_count, conflict_count))
    }

    async fn flush_run_buffered_events(
        &self,
        server_url: &str,
        bearer_token: &str,
        workspace_id: &WorkspaceId,
        run_id: &RunId,
    ) -> Result<RunFlushOutcome, TeamServerError> {
        let entries = {
            let conn = open_sqlite(&self.state.db_path)?;
            RunnerBufferStore::list_unflushed_for_run(&conn, workspace_id, run_id)?
        };
        let mut out = RunFlushOutcome::default();
        let mut rejected_fence: Option<(RunnerId, LeaseEpoch)> = None;
        for chunk in group_contiguous_reports(&entries) {
            let fence = (chunk.report.runner_id.clone(), chunk.report.lease_epoch);
            if rejected_fence.as_ref() == Some(&fence) {
                mark_conflicted_chunk(&self.state.db_path, run_id, &chunk, &mut out)?;
                out.conflicts += 1;
                continue;
            }
            rejected_fence = None;
            match self
                .post_chunk_with_recovery(
                    server_url,
                    bearer_token,
                    workspace_id,
                    run_id,
                    &chunk.report,
                )
                .await?
            {
                ChunkPostOutcome::Committed(resp) => {
                    commit_flushed_chunk(&self.state.db_path, run_id, &resp.run, &chunk, &mut out)?;
                }
                ChunkPostOutcome::PermanentConflict => {
                    mark_conflicted_chunk(&self.state.db_path, run_id, &chunk, &mut out)?;
                    out.conflicts += 1;
                    rejected_fence = Some(fence);
                }
                ChunkPostOutcome::RunnerNotOnlineDeferred => {
                    out.conflicts += 1;
                    break;
                }
            }
        }
        Ok(out)
    }

    async fn post_chunk_with_recovery(
        &self,
        server_url: &str,
        bearer_token: &str,
        workspace_id: &WorkspaceId,
        run_id: &RunId,
        report: &RunnerRunReport,
    ) -> Result<ChunkPostOutcome, TeamServerError> {
        match self
            .post_runner_state(server_url, bearer_token, workspace_id, run_id, report)
            .await
        {
            Ok(resp) => Ok(ChunkPostOutcome::Committed(Box::new(resp))),
            Err(TeamServerError::Conflict { code, .. }) if code == "runner_not_online" => {
                match self
                    .send_heartbeat(server_url, bearer_token, workspace_id, &report.runner_id)
                    .await
                {
                    Ok(_) => match self
                        .post_runner_state(server_url, bearer_token, workspace_id, run_id, report)
                        .await
                    {
                        Ok(resp) => Ok(ChunkPostOutcome::Committed(Box::new(resp))),
                        Err(TeamServerError::Conflict { code: rc, .. })
                            if rc == "runner_not_online" =>
                        {
                            Ok(ChunkPostOutcome::RunnerNotOnlineDeferred)
                        }
                        Err(TeamServerError::Conflict { .. }) => {
                            Ok(ChunkPostOutcome::PermanentConflict)
                        }
                        Err(err) => Err(err),
                    },
                    Err(TeamServerError::Offline(msg)) => Err(TeamServerError::Offline(msg)),
                    Err(_) => Ok(ChunkPostOutcome::RunnerNotOnlineDeferred),
                }
            }
            Err(TeamServerError::Conflict { .. }) => Ok(ChunkPostOutcome::PermanentConflict),
            Err(err) => Err(err),
        }
    }

    /// Submits a fenced [`RunnerRunReport`] to `POST /api/v1/workspaces/{wid}/runs/{run_id}/runner-state`.
    ///
    /// # Errors
    /// Returns [`TeamServerError`] on fencing mismatch (`409`), unreachable runner, or offline error.
    pub async fn post_runner_state(
        &self,
        server_url: &str,
        bearer_token: &str,
        workspace_id: &WorkspaceId,
        run_id: &RunId,
        report: &RunnerRunReport,
    ) -> Result<RunOwnershipResponse, TeamServerError> {
        let url = format!(
            "{}/api/v1/workspaces/{}/runs/{}/runner-state",
            server_url.trim_end_matches('/'),
            workspace_id.as_ref(),
            run_id.as_ref()
        );
        let resp = self
            .http
            .post(&url)
            .bearer_auth(bearer_token)
            .json(report)
            .send()
            .await
            .map_err(|e| TeamServerError::Offline(e.to_string()))?;
        parse_server_json_response(resp).await
    }

    /// Lists all non-terminal runs in `workspace_id` owned exclusively by `runner_id` with an active
    /// `lease_epoch > 0` and `!runner_unreachable`.
    ///
    /// # Errors
    /// Returns [`StorageError`] on `SQLite` query failure.
    pub fn list_assigned_executable_runs(
        &self,
        workspace_id: &WorkspaceId,
        runner_id: &RunnerId,
    ) -> Result<Vec<(Run, RunOwnership)>, StorageError> {
        let conn = open_sqlite(&self.state.db_path)?;
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
            let ownerships =
                SyncStore::list_run_ownerships_for_project(&conn, &binding.project_id)?;
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
}
