use crate::state::AppState;
use crate::worktree::WorktreeManager;
use runlark_domain::{Run, Timestamp};
use runlark_local::{
    BoardProjection, ProjectStore, RunQueries, SessionStore, StorageError, open_sqlite,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Notify, watch};

/// Default solo-alpha maximum concurrently executing runs (`Running` | `CancelRequested`).
pub const DEFAULT_MAX_CONCURRENT_SLOTS: u64 = 2;

/// Snapshot of current 2-slot scheduler occupancy and queue depth.
#[derive(Debug, Clone, Serialize)]
pub struct SchedulerStatus {
    pub max_slots: u64,
    pub active_slots_used: u64,
    pub available_slots: u64,
    pub active_run_ids: Vec<String>,
    pub queued_run_ids: Vec<String>,
}

/// 2-slot run admission scheduler creating isolated per-run git worktrees on admission.
pub struct Scheduler {
    state: AppState,
    max_slots: u64,
    poll_interval: Duration,
}

impl Scheduler {
    /// Creates a scheduler enforcing the solo-alpha 2-slot concurrency limit.
    #[must_use]
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            max_slots: DEFAULT_MAX_CONCURRENT_SLOTS,
            poll_interval: Duration::from_secs(1),
        }
    }

    /// Creates a scheduler with an explicit poll interval (useful in tests).
    #[must_use]
    pub fn with_poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = poll_interval;
        self
    }

    /// Runs the reactive + fallback-tick admission loop until `shutdown` signals `true`.
    pub async fn run_loop(
        self: Arc<Self>,
        notify: Arc<Notify>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        if let Err(err) = self.admit_once().await {
            tracing::warn!("Initial scheduler admission pass failed: {err}");
        }

        loop {
            if *shutdown.borrow() {
                break;
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

            if *shutdown.borrow() {
                break;
            }

            if let Err(err) = self.admit_once().await {
                tracing::warn!("Scheduler admission pass failed: {err}");
            }
        }
    }

    /// Returns the current slot occupancy and queued run IDs.
    ///
    /// # Errors
    /// Returns [`StorageError`] if querying `SQLite` fails.
    pub fn status(&self) -> Result<SchedulerStatus, StorageError> {
        let conn = open_sqlite(&self.state.db_path)?;
        let active = RunQueries::list_executing_runs(&conn)?;
        let queued = BoardProjection::list_queued_runs_fifo(&conn)?;
        let active_slots_used = u64::try_from(active.len()).unwrap_or(u64::MAX);
        Ok(SchedulerStatus {
            max_slots: self.max_slots,
            active_slots_used,
            available_slots: self.max_slots.saturating_sub(active_slots_used),
            active_run_ids: active.into_iter().map(|r| r.id.0).collect(),
            queued_run_ids: queued.into_iter().map(|r| r.id.0).collect(),
        })
    }

    /// Admits as many `Queued` runs (in FIFO order) as free execution slots allow,
    /// creating an isolated git worktree per run and waking the outbox dispatcher.
    ///
    /// # Errors
    /// Returns [`StorageError`] on database or worktree isolation failures.
    pub async fn admit_once(&self) -> Result<Vec<Run>, StorageError> {
        self.admit_once_with_hook(|_| {}).await
    }

    async fn admit_once_with_hook<F>(&self, mut before_commit: F) -> Result<Vec<Run>, StorageError>
    where
        F: FnMut(&runlark_domain::RunId),
    {
        let _admit_guard = self.state.scheduler_admit_lock.lock().await;
        let mut admitted = Vec::new();
        let mut skipped = std::collections::HashSet::new();

        loop {
            let candidate = {
                let conn = open_sqlite(&self.state.db_path)?;
                self.select_admittable_candidate(&conn, &skipped)?
            };

            let Some((run, workspace)) = candidate else {
                break;
            };

            let run_id_clone = run.id.clone();
            let wt_workspace = workspace.clone();
            let (worktree_path, base_sha) = tokio::task::spawn_blocking(move || {
                let resolution = WorktreeManager::ensure_worktree(&wt_workspace, &run_id_clone)
                    .map_err(|e| StorageError::InvalidState(e.to_string()))?;
                let sha = resolve_head_sha(&resolution.worktree_path);
                Ok::<_, StorageError>((resolution.worktree_path, sha))
            })
            .await
            .map_err(|e| StorageError::InvalidState(format!("worktree task panicked: {e}")))??;

            let worktree_str = worktree_path.to_string_lossy().into_owned();
            let now = Timestamp(AppState::current_timestamp_ms());
            before_commit(&run.id);

            let mut conn = open_sqlite(&self.state.db_path)?;
            if let Some(admitted_run) = RunQueries::admit_queued_run(
                &mut conn,
                &run.id,
                &worktree_str,
                base_sha.as_deref(),
                self.max_slots,
                now,
            )? {
                self.state.notify_outbox();
                self.state.notify_dsh_subscriber();
                admitted.push(admitted_run);
            } else {
                let used =
                    u64::try_from(RunQueries::list_executing_runs(&conn)?.len()).unwrap_or(0);
                if used >= self.max_slots {
                    break;
                }
                skipped.insert(run.id);
            }
        }

        Ok(admitted)
    }

    fn select_admittable_candidate(
        &self,
        conn: &runlark_local::Connection,
        skipped: &std::collections::HashSet<runlark_domain::RunId>,
    ) -> Result<Option<(Run, PathBuf)>, StorageError> {
        let executing = RunQueries::list_executing_runs(conn)?;
        let used = u64::try_from(executing.len()).unwrap_or(u64::MAX);
        if used >= self.max_slots {
            return Ok(None);
        }

        for run in BoardProjection::list_queued_runs_fifo(conn)? {
            if skipped.contains(&run.id) {
                continue;
            }
            if let Some(pid) = &run.project_id {
                let Some(project) = ProjectStore::get_project(conn, pid)? else {
                    tracing::warn!(
                        run_id = %run.id,
                        project_id = %pid,
                        "Skipping queued run whose project does not exist"
                    );
                    continue;
                };
                let project_limit = u64::from(project.max_concurrent_runs).min(self.max_slots);
                let project_active = executing
                    .iter()
                    .filter(|r| r.project_id.as_ref() == Some(pid))
                    .count();
                if u64::try_from(project_active).unwrap_or(u64::MAX) >= project_limit {
                    continue;
                }
                return Ok(Some((run, PathBuf::from(project.repo_root))));
            }
            let workspace = SessionStore::get_session(conn, &run.session_id)?
                .map_or_else(|| PathBuf::from("."), |s| PathBuf::from(s.cwd));
            return Ok(Some((run, workspace)));
        }
        Ok(None)
    }
}

fn resolve_head_sha(dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!sha.is_empty()).then_some(sha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use runlark_domain::{
        BudgetPolicy, CostSummary, Revision, RunId, RunStatus, Session, SessionId, SessionStatus,
        TaskId, TaskKind, WorkflowStage,
    };
    use runlark_local::initialize_schema;
    use tempfile::TempDir;

    #[tokio::test]
    async fn admit_once_continues_after_single_run_loses_queued_status() {
        let temp = TempDir::new().expect("temp dir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir");
        for args in [
            vec!["init"],
            vec!["config", "user.email", "t@runlark.dev"],
            vec!["config", "user.name", "Test"],
        ] {
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(&repo)
                    .args(&args)
                    .output()
                    .expect("git")
                    .status
                    .success()
            );
        }
        std::fs::write(repo.join("README.md"), "# R\n").expect("write");
        for args in [vec!["add", "README.md"], vec!["commit", "-m", "init"]] {
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(&repo)
                    .args(&args)
                    .output()
                    .expect("git")
                    .status
                    .success()
            );
        }

        let db_path = temp.path().join("sched_unit.db");
        let conn = open_sqlite(&db_path).expect("open");
        initialize_schema(&conn).expect("schema");
        let repo_str = repo.to_string_lossy().into_owned();
        for n in 1..=2 {
            let sid = SessionId::from(format!("sess_u_{n}"));
            let rid = RunId::from(format!("run_u_{n}"));
            SessionStore::create_session(
                &conn,
                &Session {
                    id: sid.clone(),
                    task_id: TaskId::from(format!("task_u_{n}")),
                    cwd: repo_str.clone(),
                    status: SessionStatus::Active,
                    current_revision: Revision(1),
                    active_run_id: Some(rid.clone()),
                    created_at: Timestamp(n * 10),
                    updated_at: Timestamp(n * 10),
                },
            )
            .expect("session");
            let run = Run {
                id: rid,
                session_id: sid,
                status: RunStatus::Queued,
                worktree_path: String::new(),
                revision: Revision(1),
                created_at: Timestamp(n * 10),
                updated_at: Timestamp(n * 10),
                task_id: None,
                project_id: None,
                workflow_stage: WorkflowStage::default(),
                task_kind: TaskKind::default(),
                workflow_digest: None,
                base_sha: None,
                head_sha: None,
                plan_version: 1,
                policy_digest: None,
                budget_policy: BudgetPolicy::default(),
                cost_summary: CostSummary::default(),
                repair_attempts: 0,
                pr_url: None,
            };
            SessionStore::save_run(&conn, &run).expect("run");
        }
        drop(conn);

        let scheduler = Scheduler::new(AppState::new(db_path.clone()));
        let admitted = scheduler
            .admit_once_with_hook(|rid| {
                if rid.0 == "run_u_1" {
                    let c = open_sqlite(&db_path).expect("open hook");
                    c.execute(
                        "UPDATE runs SET status = 'stopped' WHERE run_id = 'run_u_1'",
                        [],
                    )
                    .expect("stop run_u_1 before commit");
                }
            })
            .await
            .expect("admit_once");

        assert_eq!(admitted.len(), 1);
        assert_eq!(admitted[0].id.0, "run_u_2");
    }

    #[tokio::test]
    async fn admit_once_skips_queued_run_with_missing_project_record() {
        let temp = TempDir::new().expect("temp dir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir");
        for args in [
            vec!["init"],
            vec!["config", "user.email", "t@runlark.dev"],
            vec!["config", "user.name", "Test"],
        ] {
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(&repo)
                    .args(&args)
                    .output()
                    .expect("git")
                    .status
                    .success()
            );
        }
        std::fs::write(repo.join("README.md"), "# R\n").expect("write");
        for args in [vec!["add", "README.md"], vec!["commit", "-m", "init"]] {
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(&repo)
                    .args(&args)
                    .output()
                    .expect("git")
                    .status
                    .success()
            );
        }

        let db_path = temp.path().join("sched_missing_proj.db");
        let conn = open_sqlite(&db_path).expect("open");
        initialize_schema(&conn).expect("schema");
        let repo_str = repo.to_string_lossy().into_owned();
        for (n, proj) in [(1, Some("proj_deleted")), (2, None)] {
            let sid = SessionId::from(format!("sess_mp_{n}"));
            let rid = RunId::from(format!("run_mp_{n}"));
            SessionStore::create_session(
                &conn,
                &Session {
                    id: sid.clone(),
                    task_id: TaskId::from(format!("task_mp_{n}")),
                    cwd: repo_str.clone(),
                    status: SessionStatus::Active,
                    current_revision: Revision(1),
                    active_run_id: Some(rid.clone()),
                    created_at: Timestamp(n * 10),
                    updated_at: Timestamp(n * 10),
                },
            )
            .expect("session");
            let run = Run {
                id: rid,
                session_id: sid,
                status: RunStatus::Queued,
                worktree_path: String::new(),
                revision: Revision(1),
                created_at: Timestamp(n * 10),
                updated_at: Timestamp(n * 10),
                task_id: None,
                project_id: proj.map(runlark_domain::ProjectId::from),
                workflow_stage: WorkflowStage::default(),
                task_kind: TaskKind::default(),
                workflow_digest: None,
                base_sha: None,
                head_sha: None,
                plan_version: 1,
                policy_digest: None,
                budget_policy: BudgetPolicy::default(),
                cost_summary: CostSummary::default(),
                repair_attempts: 0,
                pr_url: None,
            };
            SessionStore::save_run(&conn, &run).expect("run");
        }
        drop(conn);

        let scheduler = Scheduler::new(AppState::new(db_path));
        let admitted = scheduler.admit_once().await.expect("admit_once");
        assert_eq!(admitted.len(), 1);
        assert_eq!(admitted[0].id.0, "run_mp_2");
    }
}
