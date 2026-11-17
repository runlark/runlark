use super::{EnqueueTaskRunOutcome, EnqueueTaskRunParams, RunQueries};
use crate::enum_codec::sha256_hex;
use crate::error::StorageError;
use crate::event_store::EventStore;
use crate::project_store::ProjectStore;
use crate::session_store::SessionStore;
use crate::task_store::TaskStore;
use runlark_domain::{
    ArtifactDigest, CommandId, CostSummary, DomainEvent, Revision, Run, RunId, RunStatus,
    SendPromptCommand, Session, SessionCommand, SessionId, SessionStatus, TaskId, Timestamp,
    TransitionError, start_task_run,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

impl RunQueries {
    /// Atomically checks idempotency, creates/updates the task's session, creates a `Queued`
    /// run pinned to `workflow_digest`, transitions the task to `InProgress`, and persists
    /// the initial `SendPrompt` outbox command before dispatch.
    ///
    /// # Errors
    /// Returns [`StorageError`] on payload conflict, missing project/task, revision mismatch,
    /// active run conflict, or `SQLite` error.
    pub fn enqueue_task_run(
        conn: &mut Connection,
        params: &EnqueueTaskRunParams,
        now: Timestamp,
    ) -> Result<EnqueueTaskRunOutcome, StorageError> {
        let payload_json = serde_json::to_string(params)?;
        let payload_hash = sha256_hex(payload_json.as_bytes());
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        if let Some(outcome) = Self::check_existing_enqueue(&tx, params, &payload_hash)? {
            tx.rollback()?;
            return Ok(outcome);
        }

        let outcome = Self::execute_enqueue_tx(&tx, params, &payload_json, &payload_hash, now)?;
        tx.commit()?;
        Ok(outcome)
    }

    fn check_existing_enqueue(
        tx: &rusqlite::Transaction<'_>,
        params: &EnqueueTaskRunParams,
        payload_hash: &str,
    ) -> Result<Option<EnqueueTaskRunOutcome>, StorageError> {
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT payload_hash, status FROM board_commands WHERE command_id = ?1",
                params![params.command_id.as_ref()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((existing_hash, status)) = existing else {
            return Ok(None);
        };
        if existing_hash != payload_hash {
            return Err(StorageError::CommandPayloadConflict(
                params.command_id.clone(),
            ));
        }
        let task = TaskStore::get_task(tx, &params.task_id)?
            .ok_or_else(|| StorageError::TaskNotFound(params.task_id.clone()))?;
        let run = SessionStore::get_run(tx, &params.run_id)?;
        Ok(Some(EnqueueTaskRunOutcome::AlreadyExists {
            status,
            task,
            run,
        }))
    }

    fn execute_enqueue_tx(
        tx: &rusqlite::Transaction<'_>,
        params: &EnqueueTaskRunParams,
        payload_json: &str,
        payload_hash: &str,
        now: Timestamp,
    ) -> Result<EnqueueTaskRunOutcome, StorageError> {
        let task = TaskStore::get_task(tx, &params.task_id)?
            .ok_or_else(|| StorageError::TaskNotFound(params.task_id.clone()))?;
        let project = ProjectStore::get_project(tx, &task.project_id)?
            .ok_or_else(|| StorageError::ProjectNotFound(task.project_id.clone()))?;
        let has_executing = match &task.active_run_id {
            Some(rid) => SessionStore::get_run(tx, rid)?.is_some_and(|r| !r.status.is_terminal()),
            None => false,
        };
        let expected_rev = params.expected_revision.unwrap_or(task.revision);

        let existing_session = match &task.active_session_id {
            Some(sid) => SessionStore::get_session(tx, sid)?,
            None => SessionStore::get_session(tx, &params.session_id)?,
        };
        let session_id = existing_session
            .as_ref()
            .map_or_else(|| params.session_id.clone(), |s| s.id.clone());
        let outcome = start_task_run(
            &task,
            &session_id,
            &params.run_id,
            has_executing,
            expected_rev,
            now,
        )?;
        let mut session = Self::upsert_task_session(
            tx,
            existing_session,
            &session_id,
            &task.id,
            &project.repo_root,
            &params.run_id,
            now,
        )?;

        let run = Run {
            id: params.run_id.clone(),
            session_id: session.id.clone(),
            status: RunStatus::Queued,
            worktree_path: String::new(),
            revision: Revision(1),
            created_at: now,
            updated_at: now,
            task_id: Some(task.id.clone()),
            project_id: Some(project.id),
            workflow_stage: outcome.new_task.workflow_stage,
            task_kind: task.kind,
            workflow_digest: params.workflow_digest.clone(),
            base_sha: params.base_sha.clone(),
            head_sha: None,
            plan_version: task.plan_version,
            policy_digest: Some(project.policy_digest),
            budget_policy: task
                .budget_override
                .clone()
                .unwrap_or(project.budget_policy),
            cost_summary: CostSummary::default(),
            repair_attempts: 0,
            pr_url: None,
        };
        SessionStore::save_run(tx, &run)?;
        TaskStore::update_task_with_revision(tx, &outcome.new_task, task.revision)?;
        Self::insert_start_prompt_outbox(tx, &mut session, &run.id, params, now)?;

        let mut events = Vec::new();
        let created_ev = DomainEvent::RunCreated {
            run_id: run.id.clone(),
            session_id: session.id.clone(),
            worktree_path: run.worktree_path.clone(),
            timestamp: now,
        };
        events.push(EventStore::append_domain_event(
            tx,
            &run.id,
            &created_ev,
            now,
        )?);
        for ev in &outcome.events {
            events.push(EventStore::append_domain_event(tx, &run.id, ev, now)?);
        }

        tx.execute(
            concat!(
                "INSERT INTO board_commands (",
                "command_id, aggregate_type, aggregate_id, command_type, payload, ",
                "payload_hash, expected_revision, status, created_at, updated_at) ",
                "VALUES (?1, 'task', ?2, 'start_task_run', ?3, ?4, ?5, 'acked', ?6, ?6)"
            ),
            params![
                params.command_id.as_ref(),
                params.task_id.as_ref(),
                payload_json,
                payload_hash,
                Self::to_i64(expected_rev.0)?,
                Self::to_i64(now.0)?,
            ],
        )?;
        Ok(EnqueueTaskRunOutcome::Enqueued {
            task: outcome.new_task,
            session,
            run,
            events,
        })
    }

    fn upsert_task_session(
        tx: &rusqlite::Transaction<'_>,
        existing_session: Option<Session>,
        session_id: &SessionId,
        task_id: &TaskId,
        repo_root: &str,
        run_id: &RunId,
        now: Timestamp,
    ) -> Result<Session, StorageError> {
        match existing_session {
            Some(s) if s.status == SessionStatus::Archived => Err(StorageError::Transition(
                TransitionError::SessionAlreadyArchived,
            )),
            Some(mut s) => {
                s.status = SessionStatus::Active;
                s.active_run_id = Some(run_id.clone());
                s.current_revision = Revision(s.current_revision.0 + 1);
                s.updated_at = now;
                SessionStore::update_session(tx, &s)?;
                Ok(s)
            }
            None => {
                let s = Session {
                    id: session_id.clone(),
                    task_id: task_id.clone(),
                    cwd: repo_root.to_string(),
                    status: SessionStatus::Active,
                    current_revision: Revision(1),
                    active_run_id: Some(run_id.clone()),
                    created_at: now,
                    updated_at: now,
                };
                SessionStore::create_session(tx, &s)?;
                Ok(s)
            }
        }
    }

    fn insert_start_prompt_outbox(
        tx: &rusqlite::Transaction<'_>,
        session: &mut Session,
        run_id: &RunId,
        params: &EnqueueTaskRunParams,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let send_cmd = SessionCommand::SendPrompt(SendPromptCommand {
            command_id: CommandId::from(format!("start_{}", params.command_id.as_ref())),
            session_id: session.id.clone(),
            run_id: run_id.clone(),
            prompt: params.prompt.clone(),
            expected_revision: session.current_revision,
            created_at: now,
        });
        let send_payload = serde_json::to_string(&send_cmd)?;
        let send_hash = sha256_hex(send_payload.as_bytes());
        tx.execute(
            concat!(
                "INSERT INTO commands (",
                "command_id, session_id, run_id, command_type, payload, ",
                "payload_hash, expected_revision, status, created_at, updated_at) ",
                "VALUES (?1, ?2, ?3, 'send_prompt', ?4, ?5, ?6, 'pending', ?7, ?7)"
            ),
            params![
                send_cmd.command_id().as_ref(),
                session.id.as_ref(),
                run_id.as_ref(),
                send_payload,
                send_hash,
                Self::to_i64(session.current_revision.0)?,
                Self::to_i64(now.0)?,
            ],
        )?;
        session.current_revision = Revision(session.current_revision.0 + 1);
        SessionStore::update_session(tx, session)?;
        Ok(())
    }

    /// Atomically transitions a `Queued` run to `Running`, enforces the concurrency slot cap and
    /// unique worktree path invariant across all executing runs, and persists `worktree_path`.
    ///
    /// Returns `Ok(None)` if the slot cap is currently full or the run is no longer `Queued`.
    ///
    /// # Errors
    /// Returns [`StorageError::InvalidState`] if another executing run already owns `worktree_path`,
    /// or [`StorageError`] if SQL operations fail.
    pub fn admit_queued_run(
        conn: &mut Connection,
        run_id: &RunId,
        worktree_path: &str,
        base_sha: Option<&str>,
        max_global_slots: u64,
        now: Timestamp,
    ) -> Result<Option<Run>, StorageError> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let executing = Self::list_executing_runs(&tx)?;
        if u64::try_from(executing.len()).unwrap_or(u64::MAX) >= max_global_slots {
            tx.rollback()?;
            return Ok(None);
        }
        if executing.iter().any(|r| r.worktree_path == worktree_path) {
            tx.rollback()?;
            return Err(StorageError::InvalidState(format!(
                "worktree path collision for {worktree_path}"
            )));
        }

        let Some(mut run) = SessionStore::get_run(&tx, run_id)? else {
            tx.rollback()?;
            return Ok(None);
        };
        if run.status != RunStatus::Queued {
            tx.rollback()?;
            return Ok(None);
        }

        if let Some(pid) = &run.project_id
            && let Some(project) = ProjectStore::get_project(&tx, pid)?
        {
            let project_limit = u64::from(project.max_concurrent_runs).min(max_global_slots);
            let project_active = executing
                .iter()
                .filter(|r| r.project_id.as_ref() == Some(pid))
                .count();
            if u64::try_from(project_active).unwrap_or(u64::MAX) >= project_limit {
                tx.rollback()?;
                return Ok(None);
            }
        }

        run.status = RunStatus::Running;
        run.worktree_path = worktree_path.to_string();
        if run.base_sha.is_none() {
            run.base_sha = base_sha.map(str::to_string);
        }
        run.revision = Revision(run.revision.0 + 1);
        run.updated_at = now;

        SessionStore::save_run(&tx, &run)?;
        SessionStore::update_run_worktree(&tx, &run.id, worktree_path, now)?;
        let wire_payload = serde_json::json!({
            "run_id": run.id.as_ref(),
            "worktree_path": worktree_path,
            "workflow_digest": run.workflow_digest.as_ref().map(ArtifactDigest::as_str),
        })
        .to_string();
        EventStore::append_wire_event(&tx, &run.id, "RunAdmitted", &wire_payload, now)?;

        tx.commit()?;
        Ok(Some(run))
    }
}
