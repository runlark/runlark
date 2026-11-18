use crate::runner_agent::RunnerAgentWorker;
use crate::state::AppState;
use crate::team_http;
pub use crate::team_http::{TeamServerError, parse_server_json_response};
use reqwest::Client;
use runlark_domain::{
    ArtifactDigest, ArtifactRef, CommandId, CursorSeq, ProjectId, ProjectOwnershipMode, Revision,
    RunId, TeamCommand, Timestamp, WorkspaceId, WorkspaceSnapshot,
};
use runlark_local::{
    StorageError, SyncOutboxEntry, SyncOutboxStatus, SyncOutboxStore, SyncStore, TeamAuthStore,
    open_sqlite, sha256_hex,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Notify, watch};

/// Response payload returned by `POST /api/v1/workspaces/{wid}/commands`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamCommandSubmissionResult {
    pub command_id: String,
    pub idempotent_replay: bool,
    pub cursor_seq: u64,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub revision: u64,
}

/// Summary counters returned by a team sync pass.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamSyncSummary {
    pub workspaces_synced: usize,
    pub snapshots_applied: usize,
    pub events_applied: usize,
    pub outbox_acked: usize,
    pub outbox_conflicts: usize,
    pub cas_uploaded: usize,
    pub cas_downloaded: usize,
}

/// Background and on-demand worker synchronizing `TeamManaged` workspaces, `sync_outbox`,
/// `offline_drafts`, and CAS artifacts with `runlark-server`.
#[derive(Clone)]
pub struct TeamSyncWorker {
    pub(crate) state: AppState,
    pub(crate) http: Client,
    poll_interval: Duration,
}

impl TeamSyncWorker {
    /// Creates a [`TeamSyncWorker`] bound to `state`.
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

    /// Sets a custom background poll interval (useful in tests).
    #[must_use]
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Returns a reference to the shared HTTP client.
    #[must_use]
    pub fn http(&self) -> &Client {
        &self.http
    }

    /// Runs the background sync loop until `shutdown` signals `true`.
    pub async fn run_loop(
        self: Arc<Self>,
        notify: Arc<Notify>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        loop {
            if *shutdown.borrow() {
                break;
            }
            if let Err(err) = self.sync_once().await {
                tracing::debug!("Background team sync pass skipped/deferred: {err}");
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

    /// Syncs every configured `TeamManaged` workspace once.
    ///
    /// # Errors
    /// Returns [`TeamServerError`] if reading local storage fails or syncing a workspace errors.
    pub async fn sync_once(&self) -> Result<TeamSyncSummary, TeamServerError> {
        let targets = self.collect_sync_targets()?;
        let mut total = TeamSyncSummary::default();
        for (server_url, bearer_token, workspace_id) in targets {
            let sub = self
                .sync_workspace(&server_url, &bearer_token, &workspace_id)
                .await?;
            total.workspaces_synced += 1;
            total.snapshots_applied += sub.snapshots_applied;
            total.events_applied += sub.events_applied;
            total.outbox_acked += sub.outbox_acked;
            total.outbox_conflicts += sub.outbox_conflicts;
            total.cas_uploaded += sub.cas_uploaded;
            total.cas_downloaded += sub.cas_downloaded;
        }
        Ok(total)
    }

    fn collect_sync_targets(&self) -> Result<Vec<(String, String, WorkspaceId)>, TeamServerError> {
        let conn = open_sqlite(&self.state.db_path)?;
        let mut seen = HashSet::new();
        let mut targets = Vec::new();

        for binding in SyncStore::list_project_bindings(&conn)? {
            if let ProjectOwnershipMode::TeamManaged {
                workspace_id,
                server_url,
                ..
            } = binding.ownership_mode
                && seen.insert((server_url.clone(), workspace_id.clone()))
                && let Some(auth) = TeamAuthStore::get_by_server(&conn, &server_url)?
            {
                targets.push((auth.server_url, auth.bearer_token, workspace_id));
            }
        }
        for ws in SyncStore::list_workspaces(&conn)? {
            if seen.insert((ws.server_url.clone(), ws.workspace.id.clone()))
                && let Some(auth) = TeamAuthStore::get_by_server(&conn, &ws.server_url)?
            {
                targets.push((auth.server_url, auth.bearer_token, ws.workspace.id));
            }
        }
        Ok(targets)
    }

    /// Synchronizes a single `workspace_id` against `server_url`.
    ///
    /// # Errors
    /// Returns [`TeamServerError`] if the server is offline, returns an error, or `SQLite` fails.
    pub async fn sync_workspace(
        &self,
        server_url: &str,
        bearer_token: &str,
        workspace_id: &WorkspaceId,
    ) -> Result<TeamSyncSummary, TeamServerError> {
        let mut summary = TeamSyncSummary {
            workspaces_synced: 1,
            ..TeamSyncSummary::default()
        };
        self.heartbeat_workspace_runner(server_url, bearer_token, workspace_id)
            .await?;
        let _ = RunnerAgentWorker::new(self.state.clone())
            .flush_buffered_events(server_url, bearer_token, workspace_id)
            .await?;
        summary.cas_uploaded = self
            .upload_cas_for_workspace(server_url, bearer_token, workspace_id)
            .await?;
        let (acked, conflicts) = self
            .flush_outbox_for_workspace(server_url, bearer_token, workspace_id)
            .await?;
        summary.outbox_acked = acked;
        summary.outbox_conflicts = conflicts;

        let current_cursor = {
            let conn = open_sqlite(&self.state.db_path)?;
            SyncStore::get_workspace_cursor(&conn, workspace_id)?
        };
        if current_cursor.0 == 0 {
            self.pull_and_apply_snapshot(server_url, bearer_token, workspace_id)
                .await?;
            summary.snapshots_applied += 1;
        } else {
            match self
                .tail_events_or_resnapshot(server_url, bearer_token, workspace_id, current_cursor)
                .await?
            {
                TailOutcome::Events(count) => summary.events_applied += count,
                TailOutcome::Resnapshotted => summary.snapshots_applied += 1,
            }
        }

        summary.cas_downloaded = self
            .download_cas_for_workspace(server_url, bearer_token, workspace_id)
            .await?;
        Ok(summary)
    }

    async fn heartbeat_workspace_runner(
        &self,
        server_url: &str,
        bearer_token: &str,
        workspace_id: &WorkspaceId,
    ) -> Result<(), TeamServerError> {
        let runner_id = {
            let conn = open_sqlite(&self.state.db_path)?;
            TeamAuthStore::resolve_for_workspace(&conn, Some(workspace_id))?
                .and_then(|s| s.runner_id)
        };
        if let Some(rid) = runner_id {
            match RunnerAgentWorker::new(self.state.clone())
                .send_heartbeat(server_url, bearer_token, workspace_id, &rid)
                .await
            {
                Ok(_) => {}
                Err(TeamServerError::Offline(msg)) => return Err(TeamServerError::Offline(msg)),
                Err(err) => {
                    tracing::debug!(runner_id = %rid, "Pre-sync runner heartbeat deferred: {err}");
                }
            }
        }
        Ok(())
    }

    async fn tail_events_or_resnapshot(
        &self,
        server_url: &str,
        bearer_token: &str,
        workspace_id: &WorkspaceId,
        mut cursor: CursorSeq,
    ) -> Result<TailOutcome, TeamServerError> {
        let mut total_events = 0_usize;
        loop {
            let events = team_http::fetch_events(
                &self.http,
                server_url,
                bearer_token,
                workspace_id,
                cursor,
                200,
            )
            .await?;
            if events.is_empty() {
                return Ok(TailOutcome::Events(total_events));
            }
            let count = events.len();
            let now = Timestamp(AppState::current_timestamp_ms());
            let apply_res = {
                let mut conn = open_sqlite(&self.state.db_path)?;
                SyncStore::apply_events_batch_atomic(
                    &mut conn,
                    workspace_id,
                    server_url,
                    &events,
                    now,
                )
            };
            match apply_res {
                Ok(next_cursor) => {
                    total_events += count;
                    cursor = next_cursor;
                    if count < 200 {
                        return Ok(TailOutcome::Events(total_events));
                    }
                }
                Err(StorageError::CursorGap { expected, actual }) => {
                    tracing::warn!(
                        workspace_id = %workspace_id,
                        expected = expected.0,
                        actual = actual.0,
                        "Detected workspace cursor gap; falling back to full snapshot"
                    );
                    self.pull_and_apply_snapshot(server_url, bearer_token, workspace_id)
                        .await?;
                    return Ok(TailOutcome::Resnapshotted);
                }
                Err(err) => return Err(err.into()),
            }
        }
    }

    async fn pull_and_apply_snapshot(
        &self,
        server_url: &str,
        bearer_token: &str,
        workspace_id: &WorkspaceId,
    ) -> Result<(), TeamServerError> {
        let snapshot =
            team_http::fetch_snapshot(&self.http, server_url, bearer_token, workspace_id).await?;
        let now = Timestamp(AppState::current_timestamp_ms());
        let mut conn = open_sqlite(&self.state.db_path)?;
        SyncStore::apply_snapshot_atomic(&mut conn, server_url, &snapshot, now)?;
        Ok(())
    }

    /// Flushes `pending` entries in `sync_outbox` for `workspace_id`, surfacing `409 Conflict`
    /// visibly on both `sync_outbox` and any linked `offline_drafts` instead of dropping/overwriting.
    ///
    /// # Errors
    /// Returns [`TeamServerError::Offline`] if transport fails (preserving `pending` status).
    pub async fn flush_outbox_for_workspace(
        &self,
        server_url: &str,
        bearer_token: &str,
        workspace_id: &WorkspaceId,
    ) -> Result<(usize, usize), TeamServerError> {
        let pending = {
            let conn = open_sqlite(&self.state.db_path)?;
            SyncOutboxStore::list_pending_for_workspace(&conn, workspace_id)?
        };
        let mut acked = 0_usize;
        let mut conflicts = 0_usize;

        for entry in pending {
            let command: TeamCommand = match serde_json::from_str(&entry.payload_json) {
                Ok(cmd) => cmd,
                Err(err) => {
                    let mut conn = open_sqlite(&self.state.db_path)?;
                    SyncOutboxStore::mark_failed(
                        &mut conn,
                        &entry.command_id,
                        &format!("invalid queued command JSON: {err}"),
                        Timestamp(AppState::current_timestamp_ms()),
                    )?;
                    continue;
                }
            };
            match team_http::post_team_command(
                &self.http,
                server_url,
                bearer_token,
                workspace_id,
                &command,
            )
            .await
            {
                Ok(_) => {
                    let mut conn = open_sqlite(&self.state.db_path)?;
                    SyncOutboxStore::mark_acked(
                        &mut conn,
                        &entry.command_id,
                        Timestamp(AppState::current_timestamp_ms()),
                    )?;
                    acked += 1;
                }
                Err(TeamServerError::Conflict {
                    message,
                    current_revision,
                    ..
                }) => {
                    let mut conn = open_sqlite(&self.state.db_path)?;
                    SyncOutboxStore::mark_conflict(
                        &mut conn,
                        &entry.command_id,
                        current_revision.map(Revision),
                        &message,
                        Timestamp(AppState::current_timestamp_ms()),
                    )?;
                    conflicts += 1;
                }
                Err(
                    TeamServerError::BadRequest(msg)
                    | TeamServerError::Forbidden(msg)
                    | TeamServerError::NotFound(msg),
                ) => {
                    let mut conn = open_sqlite(&self.state.db_path)?;
                    SyncOutboxStore::mark_failed(
                        &mut conn,
                        &entry.command_id,
                        &msg,
                        Timestamp(AppState::current_timestamp_ms()),
                    )?;
                    conflicts += 1;
                }
                Err(TeamServerError::Offline(msg)) => {
                    return Err(TeamServerError::Offline(msg));
                }
                Err(err) => return Err(err),
            }
        }
        Ok((acked, conflicts))
    }

    /// Submits a single [`TeamCommand`] directly to `runlark-server` and immediately syncs
    /// workspace events into local `SQLite`.
    ///
    /// # Errors
    /// Returns [`TeamServerError`] if the server rejects the command or is offline.
    pub async fn submit_command_and_sync(
        &self,
        server_url: &str,
        bearer_token: &str,
        workspace_id: &WorkspaceId,
        command: &TeamCommand,
    ) -> Result<TeamCommandSubmissionResult, TeamServerError> {
        let res = team_http::post_team_command(
            &self.http,
            server_url,
            bearer_token,
            workspace_id,
            command,
        )
        .await?;
        let _ = self
            .sync_workspace(server_url, bearer_token, workspace_id)
            .await?;
        Ok(res)
    }

    /// Fetches a consistent [`WorkspaceSnapshot`] from `runlark-server`.
    ///
    /// # Errors
    /// Returns [`TeamServerError`] on HTTP/offline failure.
    pub async fn fetch_snapshot(
        &self,
        server_url: &str,
        bearer_token: &str,
        workspace_id: &WorkspaceId,
    ) -> Result<WorkspaceSnapshot, TeamServerError> {
        team_http::fetch_snapshot(&self.http, server_url, bearer_token, workspace_id).await
    }

    /// Uploads a verified CAS blob to `runlark-server`.
    ///
    /// # Errors
    /// Returns [`TeamServerError`] on HTTP/offline failure.
    pub async fn upload_artifact(
        &self,
        server_url: &str,
        bearer_token: &str,
        workspace_id: &WorkspaceId,
        meta: &ArtifactRef,
        bytes: &[u8],
    ) -> Result<ArtifactRef, TeamServerError> {
        team_http::upload_artifact(
            &self.http,
            server_url,
            bearer_token,
            workspace_id,
            meta,
            bytes,
        )
        .await
    }

    /// Downloads and SHA-256-verifies a CAS blob from `runlark-server`.
    ///
    /// # Errors
    /// Returns [`TeamServerError`] on HTTP/offline failure or digest corruption.
    pub async fn download_artifact(
        &self,
        server_url: &str,
        bearer_token: &str,
        workspace_id: &WorkspaceId,
        digest: &ArtifactDigest,
    ) -> Result<Vec<u8>, TeamServerError> {
        team_http::download_artifact(&self.http, server_url, bearer_token, workspace_id, digest)
            .await
    }

    /// Derives a deterministic `CommandId` for `AttachArtifact` so retries are idempotent.
    #[must_use]
    pub fn deterministic_attach_command_id(
        run_id: &RunId,
        digest: &ArtifactDigest,
        label: &str,
    ) -> CommandId {
        let key = format!("{}\0{}\0{}", run_id.as_ref(), digest.as_str(), label);
        let hash = sha256_hex(key.as_bytes());
        CommandId::from(format!("cmd_att_{}", &hash[..24]))
    }

    /// Builds a [`SyncOutboxEntry`] from a [`TeamCommand`].
    ///
    /// # Errors
    /// Returns [`TeamServerError::Json`] if serialization fails.
    pub fn build_outbox_entry(
        workspace_id: &WorkspaceId,
        project_id: Option<&ProjectId>,
        draft_id: Option<&runlark_domain::DraftId>,
        command: &TeamCommand,
        expected_revision: Option<Revision>,
        now: Timestamp,
    ) -> Result<SyncOutboxEntry, TeamServerError> {
        let command_type = match command {
            TeamCommand::CreateTask(_) => "create_task",
            TeamCommand::MoveTaskStage(_) => "move_task_stage",
            TeamCommand::UpdateTaskPlan(_) => "update_task_plan",
            TeamCommand::CreateTeamRun(_) => "create_team_run",
            TeamCommand::AssignRunner(_) => "assign_runner",
            TeamCommand::CancelRun(_) => "cancel_run",
            TeamCommand::ForceCancelUnreachableRun(_) => "force_cancel_unreachable_run",
            TeamCommand::AddComment(_) => "add_comment",
            TeamCommand::RecordAttempt(_) => "record_attempt",
            TeamCommand::RecordDecision(_) => "record_decision",
            TeamCommand::AttachArtifact(_) => "attach_artifact",
        };
        Ok(SyncOutboxEntry {
            command_id: command.command_id().clone(),
            workspace_id: workspace_id.clone(),
            project_id: project_id.cloned(),
            draft_id: draft_id.cloned(),
            command_type: command_type.to_string(),
            payload_json: serde_json::to_string(command)?,
            expected_revision,
            status: SyncOutboxStatus::Pending,
            last_error: None,
            conflict_server_revision: None,
            created_at: now,
            updated_at: now,
        })
    }
}

enum TailOutcome {
    Events(usize),
    Resnapshotted,
}
