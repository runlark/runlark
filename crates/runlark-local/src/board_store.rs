use crate::attempt_store::AttemptStore;
use crate::board_projection::BoardProjection;
use crate::cas_store::ArtifactCasStore;
use crate::connection::to_sql_i64;
use crate::decision_store::DecisionStore;
use crate::enum_codec::{artifact_kind_to_str, sha256_hex};
use crate::error::StorageError;
use crate::event_store::{EventStore, StoredEvent};
use crate::project_store::ProjectStore;
use crate::session_store::SessionStore;
use crate::task_store::TaskStore;
use runlark_domain::{
    ArtifactKind, ArtifactRef, AttachArtifactCommand, Attempt, BoardCommand, CommandId,
    CreateProjectCommand, CreateTaskCommand, Decision, DecisionBinding, DomainEvent,
    MoveTaskStageCommand, Project, RecordAttemptCommand, RecordDecisionCommand, Revision, RunId,
    Task, TaskStage, Timestamp, TransitionError, UpdateTaskPlanCommand, apply_decision,
    move_task_stage, record_attempt_and_budget, update_task_plan,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

/// Result of atomically applying an A1 [`BoardCommand`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardCommandOutcome {
    Applied { events: Vec<StoredEvent> },
    AlreadyExists { status: String },
}

struct CommandRecordMeta<'a> {
    command_id: &'a CommandId,
    aggregate_type: &'static str,
    aggregate_id: &'a str,
    command_type: &'static str,
    payload_json: &'a str,
    payload_hash: &'a str,
    expected_revision: Revision,
    now: Timestamp,
}

pub struct BoardStore;

impl BoardStore {
    fn command_metadata(command: &BoardCommand) -> (&'static str, &str, &'static str, Revision) {
        match command {
            BoardCommand::CreateProject(c) => (
                "project",
                c.project_id.as_ref(),
                "create_project",
                Revision(0),
            ),
            BoardCommand::CreateTask(c) => ("task", c.task_id.as_ref(), "create_task", Revision(0)),
            BoardCommand::MoveTaskStage(c) => (
                "task",
                c.task_id.as_ref(),
                "move_task_stage",
                c.expected_revision,
            ),
            BoardCommand::UpdateTaskPlan(c) => (
                "task",
                c.task_id.as_ref(),
                "update_task_plan",
                c.expected_revision,
            ),
            BoardCommand::RecordAttempt(c) => (
                "run",
                c.run_id.as_ref(),
                "record_attempt",
                c.expected_revision,
            ),
            BoardCommand::RecordDecision(c) => (
                "decision",
                c.decision_id.as_ref(),
                "record_decision",
                c.expected_revision,
            ),
            BoardCommand::AttachArtifact(c) => {
                ("run", c.run_id.as_ref(), "attach_artifact", Revision(0))
            }
        }
    }

    fn insert_board_command(
        tx: &Transaction<'_>,
        meta: &CommandRecordMeta<'_>,
    ) -> Result<(), StorageError> {
        tx.execute(
            concat!(
                "INSERT INTO board_commands (",
                "command_id, aggregate_type, aggregate_id, command_type, payload, ",
                "payload_hash, expected_revision, status, created_at, updated_at) ",
                "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'acked', ?8, ?8)"
            ),
            params![
                meta.command_id.as_ref(),
                meta.aggregate_type,
                meta.aggregate_id,
                meta.command_type,
                meta.payload_json,
                meta.payload_hash,
                to_sql_i64(meta.expected_revision.0)?,
                to_sql_i64(meta.now.0)?,
            ],
        )?;
        Ok(())
    }

    /// Fetches the stored `payload` JSON for an already-persisted `board_commands` entry, if any.
    ///
    /// # Errors
    /// Returns [`StorageError::Sqlite`] if querying `board_commands` fails.
    pub fn get_command_payload(
        conn: &Connection,
        command_id: &CommandId,
    ) -> Result<Option<String>, StorageError> {
        conn.query_row(
            "SELECT payload FROM board_commands WHERE command_id = ?1",
            params![command_id.as_ref()],
            |row| row.get(0),
        )
        .optional()
        .map_err(StorageError::from)
    }

    /// Atomically checks idempotency (`board_commands`), validates invariants, persists state,
    /// and appends `DomainEvent` records inside one `TransactionBehavior::Immediate` transaction.
    ///
    /// # Errors
    /// Returns [`StorageError::CommandPayloadConflict`] if the same `command_id` is reused with a
    /// different payload, [`StorageError::Transition`] if domain rules fail, or [`StorageError::Sqlite`].
    pub fn apply_board_command(
        conn: &mut Connection,
        command: &BoardCommand,
        now: Timestamp,
    ) -> Result<BoardCommandOutcome, StorageError> {
        let payload_json = serde_json::to_string(command)?;
        let canonical_json = serde_json::to_string(&command.canonical_for_hash())?;
        let payload_hash = sha256_hex(canonical_json.as_bytes());
        let command_id = command.command_id();

        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT payload_hash, status FROM board_commands WHERE command_id = ?1",
                params![command_id.as_ref()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        if let Some((existing_hash, status)) = existing {
            tx.rollback()?;
            if existing_hash != payload_hash {
                return Err(StorageError::CommandPayloadConflict(command_id.clone()));
            }
            return Ok(BoardCommandOutcome::AlreadyExists { status });
        }

        let (aggregate_type, aggregate_id, command_type, expected_revision) =
            Self::command_metadata(command);
        let events = Self::execute_command(&tx, command, now)?;

        let mut stored_events = Vec::with_capacity(events.len());
        for (idx, event) in events.iter().enumerate() {
            let run_id = Self::event_run_id(event)
                .unwrap_or_else(|| RunId::from(format!("board:{aggregate_id}:{idx}")));
            stored_events.push(EventStore::append_domain_event(&tx, &run_id, event, now)?);
        }

        Self::insert_board_command(
            &tx,
            &CommandRecordMeta {
                command_id,
                aggregate_type,
                aggregate_id,
                command_type,
                payload_json: &payload_json,
                payload_hash: &payload_hash,
                expected_revision,
                now,
            },
        )?;

        tx.commit()?;
        Ok(BoardCommandOutcome::Applied {
            events: stored_events,
        })
    }

    fn event_run_id(event: &DomainEvent) -> Option<RunId> {
        match event {
            DomainEvent::RunStageAdvanced { run_id, .. }
            | DomainEvent::AttemptRecorded { run_id, .. }
            | DomainEvent::RepairScheduled { run_id, .. }
            | DomainEvent::BudgetExceeded { run_id, .. }
            | DomainEvent::UnknownCostRecorded { run_id, .. }
            | DomainEvent::DecisionRecorded { run_id, .. }
            | DomainEvent::ArtifactAttached { run_id, .. }
            | DomainEvent::RunCompleted { run_id, .. } => Some(run_id.clone()),
            _ => None,
        }
    }

    fn execute_command(
        tx: &Transaction<'_>,
        command: &BoardCommand,
        now: Timestamp,
    ) -> Result<Vec<DomainEvent>, StorageError> {
        match command {
            BoardCommand::CreateProject(c) => Self::exec_create_project(tx, c, now),
            BoardCommand::CreateTask(c) => Self::exec_create_task(tx, c, now),
            BoardCommand::MoveTaskStage(c) => Self::exec_move_task(tx, c, now),
            BoardCommand::UpdateTaskPlan(c) => Self::exec_update_plan(tx, c, now),
            BoardCommand::RecordAttempt(c) => Self::exec_record_attempt(tx, c, now),
            BoardCommand::RecordDecision(c) => Self::exec_record_decision(tx, c, now),
            BoardCommand::AttachArtifact(c) => Self::exec_attach_artifact(tx, c, now),
        }
    }

    fn exec_create_project(
        tx: &Transaction<'_>,
        c: &CreateProjectCommand,
        now: Timestamp,
    ) -> Result<Vec<DomainEvent>, StorageError> {
        let project = Project::new(
            c.project_id.clone(),
            c.name.clone(),
            c.repo_root.clone(),
            c.default_branch.clone(),
            c.budget_policy.clone(),
            c.policy_digest.clone(),
            now,
        );
        ProjectStore::create_project(tx, &project)?;
        Ok(vec![DomainEvent::ProjectCreated {
            project_id: project.id,
            name: project.name,
            policy_digest: project.policy_digest,
            timestamp: now,
        }])
    }

    fn exec_create_task(
        tx: &Transaction<'_>,
        c: &CreateTaskCommand,
        now: Timestamp,
    ) -> Result<Vec<DomainEvent>, StorageError> {
        if ProjectStore::get_project(tx, &c.project_id)?.is_none() {
            return Err(StorageError::ProjectNotFound(c.project_id.clone()));
        }
        let task = Task {
            id: c.task_id.clone(),
            project_id: c.project_id.clone(),
            title: c.title.clone(),
            description: c.description.clone(),
            kind: c.kind,
            stage: TaskStage::Inbox,
            workflow_stage: c.kind.initial_workflow_stage(),
            plan_text: None,
            plan_version: 1,
            base_ref: c.base_ref.clone(),
            head_ref: None,
            pr_url: None,
            active_session_id: None,
            active_run_id: None,
            latest_run_id: None,
            budget_override: c.budget_override.clone(),
            revision: Revision(1),
            created_at: now,
            updated_at: now,
        };
        TaskStore::create_task(tx, &task)?;
        Ok(vec![DomainEvent::TaskCreated {
            task_id: task.id,
            project_id: task.project_id,
            title: task.title,
            kind: task.kind,
            timestamp: now,
        }])
    }

    fn exec_move_task(
        tx: &Transaction<'_>,
        c: &MoveTaskStageCommand,
        now: Timestamp,
    ) -> Result<Vec<DomainEvent>, StorageError> {
        let task = TaskStore::get_task(tx, &c.task_id)?
            .ok_or_else(|| StorageError::TaskNotFound(c.task_id.clone()))?;
        let has_executing = match &task.active_run_id {
            Some(run_id) => SessionStore::get_run(tx, run_id)?
                .is_some_and(|r| r.status.consumes_execution_slot()),
            None => false,
        };
        let has_accept = DecisionStore::has_accept_for_task(tx, &task.id)?;
        let outcome = move_task_stage(
            &task,
            c.target_stage,
            &c.actor,
            c.expected_revision,
            has_executing,
            has_accept,
            now,
        )?;
        TaskStore::update_task_with_revision(tx, &outcome.new_task, task.revision)?;
        Ok(outcome.events)
    }

    fn exec_update_plan(
        tx: &Transaction<'_>,
        c: &UpdateTaskPlanCommand,
        now: Timestamp,
    ) -> Result<Vec<DomainEvent>, StorageError> {
        let task = TaskStore::get_task(tx, &c.task_id)?
            .ok_or_else(|| StorageError::TaskNotFound(c.task_id.clone()))?;
        let outcome = update_task_plan(&task, c.plan_text.clone(), c.expected_revision, now)?;
        TaskStore::update_task_with_revision(tx, &outcome.new_task, task.revision)?;
        Ok(outcome.events)
    }

    fn exec_record_attempt(
        tx: &Transaction<'_>,
        c: &RecordAttemptCommand,
        now: Timestamp,
    ) -> Result<Vec<DomainEvent>, StorageError> {
        let run = SessionStore::get_run(tx, &c.run_id)?
            .ok_or_else(|| StorageError::RunNotFound(c.run_id.clone()))?;
        if run.revision != c.expected_revision {
            return Err(StorageError::Transition(
                TransitionError::RevisionMismatch {
                    expected: c.expected_revision,
                    actual: run.revision,
                },
            ));
        }
        let prior = AttemptStore::list_attempts_for_run(tx, &c.run_id)?;
        let attempt_no = u32::try_from(prior.len() + 1).unwrap_or(u32::MAX);
        let attempt = Attempt {
            id: c.attempt_id.clone(),
            run_id: c.run_id.clone(),
            attempt_no,
            workflow_stage: c.workflow_stage,
            started_at: c.created_at,
            ended_at: Some(now),
            outcome: c.outcome.clone(),
            cost_micro_usd: c.cost_micro_usd,
            unknown_cost_reason: c.unknown_cost_reason.clone(),
            repair_reason: c.repair_reason.clone(),
        };
        AttemptStore::save_attempt(tx, &attempt)?;
        let outcome = record_attempt_and_budget(&run, &prior, &attempt, now);
        SessionStore::save_run(tx, &outcome.new_run)?;
        Ok(outcome.events)
    }

    fn exec_record_decision(
        tx: &Transaction<'_>,
        c: &RecordDecisionCommand,
        now: Timestamp,
    ) -> Result<Vec<DomainEvent>, StorageError> {
        let task = TaskStore::get_task(tx, &c.task_id)?
            .ok_or_else(|| StorageError::TaskNotFound(c.task_id.clone()))?;
        if task.revision != c.expected_revision {
            return Err(StorageError::Transition(
                TransitionError::RevisionMismatch {
                    expected: c.expected_revision,
                    actual: task.revision,
                },
            ));
        }
        let run = SessionStore::get_run(tx, &c.run_id)?
            .ok_or_else(|| StorageError::RunNotFound(c.run_id.clone()))?;
        let current_head = run.head_sha.as_deref().unwrap_or(&c.head_sha);
        let current_base = run.base_sha.as_deref().unwrap_or(&c.base_sha);
        let current_policy = run.policy_digest.as_ref().unwrap_or(&c.policy_digest);
        let attached_evidence = ArtifactCasStore::list_artifacts_for_run(tx, &c.run_id)?
            .into_iter()
            .rev()
            .find(|(meta, label)| {
                meta.kind == ArtifactKind::EvidenceBundle && label == "evidence_bundle"
            })
            .map(|(meta, _)| meta.digest_sha256);
        let current_evidence = attached_evidence.as_ref().unwrap_or(&c.evidence_digest);
        let decision = Decision {
            id: c.decision_id.clone(),
            task_id: c.task_id.clone(),
            run_id: c.run_id.clone(),
            kind: c.kind,
            actor: c.actor.clone(),
            plan_version: c.plan_version,
            head_sha: c.head_sha.clone(),
            base_sha: c.base_sha.clone(),
            policy_digest: c.policy_digest.clone(),
            evidence_digest: c.evidence_digest.clone(),
            comment: c.comment.clone(),
            created_at: now,
        };
        let binding = DecisionBinding {
            plan_version: task.plan_version,
            head_sha: current_head,
            base_sha: current_base,
            policy_digest: current_policy,
            evidence_digest: current_evidence,
        };
        let outcome = apply_decision(&task, &run, &decision, &binding, now)?;
        DecisionStore::record_decision(tx, &decision)?;
        TaskStore::update_task_with_revision(tx, &outcome.new_task, task.revision)?;
        SessionStore::save_run(tx, &outcome.new_run)?;
        Ok(outcome.events)
    }

    fn exec_attach_artifact(
        tx: &Transaction<'_>,
        c: &AttachArtifactCommand,
        now: Timestamp,
    ) -> Result<Vec<DomainEvent>, StorageError> {
        if SessionStore::get_run(tx, &c.run_id)?.is_none() {
            return Err(StorageError::RunNotFound(c.run_id.clone()));
        }
        if ArtifactCasStore::get_artifact_meta(tx, &c.digest_sha256)?.is_none() {
            let meta = ArtifactRef {
                digest_sha256: c.digest_sha256.clone(),
                kind: c.kind,
                media_type: c.media_type.clone(),
                size_bytes: c.size_bytes,
                created_at: now,
            };
            tx.execute(
                concat!(
                    "INSERT OR IGNORE INTO artifacts (digest_sha256, kind, media_type, size_bytes, created_at) ",
                    "VALUES (?1, ?2, ?3, ?4, ?5)"
                ),
                params![
                    meta.digest_sha256.as_str(),
                    artifact_kind_to_str(meta.kind),
                    meta.media_type,
                    to_sql_i64(meta.size_bytes)?,
                    to_sql_i64(meta.created_at.0)?,
                ],
            )?;
        }
        ArtifactCasStore::link_to_run(tx, &c.run_id, &c.digest_sha256, &c.label, now)?;
        Ok(vec![DomainEvent::ArtifactAttached {
            run_id: c.run_id.clone(),
            digest_sha256: c.digest_sha256.clone(),
            kind: c.kind,
            label: c.label.clone(),
            timestamp: now,
        }])
    }

    /// Convenience helper forwarding to [`BoardProjection::count_executing_runs`].
    ///
    /// # Errors
    /// Returns [`StorageError`] if the SQL query fails.
    pub fn count_executing_runs(conn: &Connection) -> Result<u64, StorageError> {
        BoardProjection::count_executing_runs(conn)
    }
}
