use crate::command_store::{CommandApplyOutcome, CommandStore};
use crate::error::StorageError;
use crate::event_store::{EventStore, StoredEvent};
use crate::question_store::QuestionStore;
use crate::session_store::SessionStore;
use runlark_domain::{
    AnswerQuestionCommand, Question, QuestionId, QuestionStatus, Run, RunId, RuntimeEvent, Session,
    SessionCommand, SessionId, Timestamp, TransitionOutcome, TurnEndStatus, apply_runtime_event,
};
use rusqlite::{Connection, TransactionBehavior};

/// Outcome of atomically applying an external DSH runtime lifecycle event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeApplyOutcome {
    pub session: Session,
    pub run: Run,
    pub events: Vec<StoredEvent>,
}

/// Coordinates atomic multi-table `SQLite` transactions across `sessions`, `runs`, `commands`,
/// `questions`, and `events`.
pub struct LocalStore;

impl LocalStore {
    /// Atomically persists a command intent and applies its pure domain transition.
    ///
    /// # Errors
    /// Returns `StorageError` and rolls back the transaction on conflict, missing entity,
    /// domain transition failure, or `SQLite` error.
    pub fn apply_command(
        conn: &mut Connection,
        command: &SessionCommand,
        now: Timestamp,
    ) -> Result<CommandApplyOutcome, StorageError> {
        CommandStore::persist_command_and_transition(conn, command, now)
    }

    /// Atomically applies an external DSH runtime event (`turn/start`, `user/question`,
    /// `turn/end`) to the local `SQLite` state, updating `sessions`, `runs`, `questions`,
    /// and `events` inside a single `TransactionBehavior::Immediate` transaction.
    ///
    /// # Errors
    /// Returns `StorageError` if the session or run is missing, the domain transition is invalid,
    /// or any `SQLite` write fails.
    pub fn apply_runtime_event(
        conn: &mut Connection,
        event: &RuntimeEvent,
        now: Timestamp,
    ) -> Result<RuntimeApplyOutcome, StorageError> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let session = SessionStore::get_session(&tx, event.session_id())?
            .ok_or_else(|| StorageError::SessionNotFound(event.session_id().clone()))?;
        let run = SessionStore::get_run(&tx, event.run_id())?
            .ok_or_else(|| StorageError::RunNotFound(event.run_id().clone()))?;

        let TransitionOutcome {
            new_session,
            new_run,
            events,
        } = apply_runtime_event(&session, &run, event, now)?;

        let updated_run = new_run.ok_or_else(|| {
            StorageError::InvalidState("runtime event transition must produce a run".to_string())
        })?;

        if new_session.current_revision != session.current_revision {
            SessionStore::update_session_with_revision(
                &tx,
                &new_session,
                session.current_revision,
            )?;
        }
        SessionStore::save_run(&tx, &updated_run)?;

        match event {
            RuntimeEvent::UserQuestionAsked {
                question_id,
                prompt,
                options,
                ..
            } => {
                let question = Question {
                    id: question_id.clone(),
                    session_id: session.id.clone(),
                    run_id: updated_run.id.clone(),
                    prompt: prompt.clone(),
                    options: options.clone(),
                    status: QuestionStatus::Pending,
                    answer: None,
                    created_at: now,
                    updated_at: now,
                };
                QuestionStore::create_question(&tx, &question)?;
            }
            RuntimeEvent::TurnEnded { .. } => {
                QuestionStore::mark_cancelled_for_run(&tx, &updated_run.id, now)?;
            }
            RuntimeEvent::TurnStarted { .. } => {}
        }

        let mut stored_events = Vec::with_capacity(events.len());
        for domain_event in &events {
            stored_events.push(EventStore::append_domain_event(
                &tx,
                &updated_run.id,
                domain_event,
                now,
            )?);
        }

        tx.commit()?;
        Ok(RuntimeApplyOutcome {
            session: new_session,
            run: updated_run,
            events: stored_events,
        })
    }

    /// Reconciles a `turn/end` lifecycle event atomically (`CancelRequested -> Stopped`,
    /// or `Running -> Completed / Stopped / Failed`).
    ///
    /// # Errors
    /// Returns `StorageError` on missing entity, invalid state transition, or `SQLite` failure.
    pub fn record_turn_end(
        conn: &mut Connection,
        session_id: &SessionId,
        run_id: &RunId,
        outcome: TurnEndStatus,
        now: Timestamp,
    ) -> Result<RuntimeApplyOutcome, StorageError> {
        let event = RuntimeEvent::TurnEnded {
            session_id: session_id.clone(),
            run_id: run_id.clone(),
            outcome,
        };
        Self::apply_runtime_event(conn, &event, now)
    }

    /// Persists a DSH `user/question` pause atomically, moving the run to
    /// `WaitingForApproval` and storing the pending `Question`.
    ///
    /// # Errors
    /// Returns `StorageError` on missing entity, invalid state transition, or `SQLite` failure.
    pub fn record_user_question(
        conn: &mut Connection,
        session_id: &SessionId,
        run_id: &RunId,
        question_id: &QuestionId,
        prompt: &str,
        options: &[String],
        now: Timestamp,
    ) -> Result<RuntimeApplyOutcome, StorageError> {
        let event = RuntimeEvent::UserQuestionAsked {
            session_id: session_id.clone(),
            run_id: run_id.clone(),
            question_id: question_id.clone(),
            prompt: prompt.to_string(),
            options: options.to_vec(),
        };
        Self::apply_runtime_event(conn, &event, now)
    }

    /// Atomically answers a pending human question via `AnswerQuestionCommand`, marking the
    /// question answered and returning the run from `WaitingForApproval` to `Running`.
    ///
    /// # Errors
    /// Returns `StorageError` on conflict, missing entity, invalid state, or `SQLite` failure.
    pub fn answer_question(
        conn: &mut Connection,
        command: AnswerQuestionCommand,
        now: Timestamp,
    ) -> Result<CommandApplyOutcome, StorageError> {
        Self::apply_command(conn, &SessionCommand::AnswerQuestion(command), now)
    }
}
