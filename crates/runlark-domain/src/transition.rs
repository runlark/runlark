use crate::command::{
    AnswerQuestionCommand, ArchiveSessionCommand, ChangeCwdCommand, ResumeSessionCommand,
    SendPromptCommand, SessionCommand, StopRunCommand,
};
use crate::error::TransitionError;
use crate::event::DomainEvent;
use crate::id::{QuestionId, Revision, RunId, SessionId, Timestamp};
use crate::state::{Run, RunStatus, Session, SessionStatus};
use serde::{Deserialize, Serialize};

/// Result of applying a domain command or runtime event to a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionOutcome {
    pub new_session: Session,
    pub new_run: Option<Run>,
    pub events: Vec<DomainEvent>,
}

/// Terminal outcome reported by DSH when a turn ends (`turn/end`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnEndStatus {
    Completed,
    Stopped,
    Failed { error: String },
}

/// External runtime lifecycle events emitted by DSH for a managed run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeEvent {
    TurnStarted {
        session_id: SessionId,
        run_id: RunId,
    },
    UserQuestionAsked {
        session_id: SessionId,
        run_id: RunId,
        question_id: QuestionId,
        prompt: String,
        options: Vec<String>,
    },
    TurnEnded {
        session_id: SessionId,
        run_id: RunId,
        outcome: TurnEndStatus,
    },
}

impl RuntimeEvent {
    #[must_use]
    pub fn session_id(&self) -> &SessionId {
        match self {
            Self::TurnStarted { session_id, .. }
            | Self::UserQuestionAsked { session_id, .. }
            | Self::TurnEnded { session_id, .. } => session_id,
        }
    }

    #[must_use]
    pub fn run_id(&self) -> &RunId {
        match self {
            Self::TurnStarted { run_id, .. }
            | Self::UserQuestionAsked { run_id, .. }
            | Self::TurnEnded { run_id, .. } => run_id,
        }
    }
}

fn is_terminal_run_status(status: RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Stopped | RunStatus::Completed | RunStatus::Failed
    )
}

fn validate_active_run<'a>(
    active_run: Option<&'a Run>,
    requested_run_id: &RunId,
) -> Result<&'a Run, TransitionError> {
    let Some(run) = active_run else {
        return Err(TransitionError::NoActiveRun);
    };
    if &run.id != requested_run_id {
        return Err(TransitionError::ActiveRunMismatch {
            active: run.id.clone(),
            requested: requested_run_id.clone(),
        });
    }
    if run.status == RunStatus::CancelRequested {
        return Err(TransitionError::RunCancelling);
    }
    if is_terminal_run_status(run.status) {
        return Err(TransitionError::RunAlreadyTerminated);
    }
    Ok(run)
}

fn handle_send_prompt(
    session: &Session,
    active_run: Option<&Run>,
    cmd: &SendPromptCommand,
    now: Timestamp,
) -> Result<TransitionOutcome, TransitionError> {
    if let Some(run) = active_run {
        if run.status == RunStatus::CancelRequested {
            return Err(TransitionError::RunCancelling);
        }
        if run.status == RunStatus::WaitingForApproval {
            return Err(TransitionError::RunWaitingForApproval);
        }
        if is_terminal_run_status(run.status) {
            if run.id == cmd.run_id {
                return Err(TransitionError::RunAlreadyTerminated);
            }
        } else if run.id != cmd.run_id {
            return Err(TransitionError::ActiveRunMismatch {
                active: run.id.clone(),
                requested: cmd.run_id.clone(),
            });
        }
    }

    let mut next_session = session.clone();
    next_session.current_revision = Revision(session.current_revision.0 + 1);
    next_session.updated_at = now;

    let mut run = match active_run.filter(|r| !is_terminal_run_status(r.status)) {
        Some(existing) => {
            let mut updated = existing.clone();
            updated.revision = Revision(existing.revision.0 + 1);
            updated
        }
        None => Run {
            id: cmd.run_id.clone(),
            session_id: session.id.clone(),
            status: RunStatus::Queued,
            worktree_path: session.cwd.clone(),
            revision: Revision(1),
            created_at: now,
            updated_at: now,
        },
    };
    run.status = RunStatus::Running;
    run.updated_at = now;
    next_session.active_run_id = Some(run.id.clone());
    next_session.status = SessionStatus::Active;

    Ok(TransitionOutcome {
        new_session: next_session,
        new_run: Some(run.clone()),
        events: vec![DomainEvent::PromptEnqueued {
            run_id: run.id,
            command_id: cmd.command_id.clone(),
            prompt: cmd.prompt.clone(),
            timestamp: now,
        }],
    })
}

fn handle_stop_run(
    session: &Session,
    active_run: Option<&Run>,
    cmd: &StopRunCommand,
    now: Timestamp,
) -> Result<TransitionOutcome, TransitionError> {
    let run = validate_active_run(active_run, &cmd.run_id)?;

    let mut next_session = session.clone();
    next_session.current_revision = Revision(session.current_revision.0 + 1);
    next_session.updated_at = now;

    let mut next_run = run.clone();
    // INVARIANT: Stop does not transition directly to Stopped!
    next_run.status = RunStatus::CancelRequested;
    next_run.revision = Revision(run.revision.0 + 1);
    next_run.updated_at = now;

    Ok(TransitionOutcome {
        new_session: next_session,
        new_run: Some(next_run.clone()),
        events: vec![DomainEvent::RunCancelRequested {
            run_id: next_run.id,
            command_id: cmd.command_id.clone(),
            reason: cmd.reason.clone(),
            timestamp: now,
        }],
    })
}

fn handle_resume_session(
    session: &Session,
    active_run: Option<&Run>,
    cmd: &ResumeSessionCommand,
    now: Timestamp,
) -> Result<TransitionOutcome, TransitionError> {
    let Some(run) = active_run else {
        return Err(TransitionError::NoActiveRun);
    };
    if run.status == RunStatus::CancelRequested {
        return Err(TransitionError::RunCancelling);
    }
    if is_terminal_run_status(run.status) {
        return Err(TransitionError::RunAlreadyTerminated);
    }

    let mut next_session = session.clone();
    next_session.current_revision = Revision(session.current_revision.0 + 1);
    next_session.updated_at = now;

    let mut next_run = run.clone();
    if next_run.status == RunStatus::Interrupted {
        next_run.status = RunStatus::Running;
        next_run.revision = Revision(run.revision.0 + 1);
        next_run.updated_at = now;
    }

    Ok(TransitionOutcome {
        new_session: next_session,
        new_run: Some(next_run.clone()),
        events: vec![DomainEvent::SessionResumed {
            session_id: session.id.clone(),
            run_id: next_run.id,
            command_id: cmd.command_id.clone(),
            after_seq: cmd.after_seq,
            timestamp: now,
        }],
    })
}

fn handle_answer_question(
    session: &Session,
    active_run: Option<&Run>,
    cmd: &AnswerQuestionCommand,
    now: Timestamp,
) -> Result<TransitionOutcome, TransitionError> {
    let run = validate_active_run(active_run, &cmd.run_id)?;
    if run.status != RunStatus::WaitingForApproval {
        return Err(TransitionError::RunNotWaitingForApproval);
    }

    let mut next_session = session.clone();
    next_session.current_revision = Revision(session.current_revision.0 + 1);
    next_session.updated_at = now;
    next_session.status = SessionStatus::Active;

    let mut next_run = run.clone();
    next_run.status = RunStatus::Running;
    next_run.revision = Revision(run.revision.0 + 1);
    next_run.updated_at = now;

    Ok(TransitionOutcome {
        new_session: next_session,
        new_run: Some(next_run.clone()),
        events: vec![DomainEvent::QuestionAnswered {
            question_id: cmd.question_id.clone(),
            session_id: session.id.clone(),
            run_id: next_run.id,
            command_id: cmd.command_id.clone(),
            answer: cmd.answer.clone(),
            timestamp: now,
        }],
    })
}

fn is_run_blocking_session_mutation(status: RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Queued
            | RunStatus::Running
            | RunStatus::WaitingForApproval
            | RunStatus::CancelRequested
    )
}

// Session-level commands (ChangeCwd, ArchiveSession) do not address a specific run and ignore cmd.run_id.
fn handle_change_cwd(
    session: &Session,
    active_run: Option<&Run>,
    cmd: &ChangeCwdCommand,
    now: Timestamp,
) -> Result<TransitionOutcome, TransitionError> {
    if let Some(run) = active_run
        && is_run_blocking_session_mutation(run.status)
    {
        return Err(TransitionError::CwdConflictActiveRun);
    }

    let mut next_session = session.clone();
    next_session.current_revision = Revision(session.current_revision.0 + 1);
    next_session.updated_at = now;
    next_session.cwd.clone_from(&cmd.new_cwd);

    Ok(TransitionOutcome {
        new_session: next_session,
        new_run: active_run.cloned(),
        events: vec![DomainEvent::CwdChanged {
            session_id: session.id.clone(),
            new_cwd: cmd.new_cwd.clone(),
            timestamp: now,
        }],
    })
}

fn handle_archive_session(
    session: &Session,
    active_run: Option<&Run>,
    _cmd: &ArchiveSessionCommand,
    now: Timestamp,
) -> Result<TransitionOutcome, TransitionError> {
    if let Some(run) = active_run
        && is_run_blocking_session_mutation(run.status)
    {
        return Err(TransitionError::ArchiveBlockedByActiveRun);
    }

    let mut next_session = session.clone();
    next_session.current_revision = Revision(session.current_revision.0 + 1);
    next_session.updated_at = now;
    next_session.status = SessionStatus::Archived;
    next_session.active_run_id = None;

    Ok(TransitionOutcome {
        new_session: next_session,
        new_run: None,
        events: vec![DomainEvent::SessionArchived {
            session_id: session.id.clone(),
            timestamp: now,
        }],
    })
}

/// Applies a command to the current session and run state in a pure, deterministic manner.
///
/// # Errors
/// Returns `TransitionError` if the command violates state constraints, session identity,
/// or revision preconditions.
pub fn transition(
    session: &Session,
    active_run: Option<&Run>,
    command: &SessionCommand,
    now: Timestamp,
) -> Result<TransitionOutcome, TransitionError> {
    if command.session_id() != &session.id {
        return Err(TransitionError::SessionMismatch {
            expected: session.id.clone(),
            actual: command.session_id().clone(),
        });
    }
    if session.status == SessionStatus::Archived {
        return Err(TransitionError::SessionAlreadyArchived);
    }
    if command.expected_revision() != session.current_revision {
        return Err(TransitionError::RevisionMismatch {
            expected: command.expected_revision(),
            actual: session.current_revision,
        });
    }

    match command {
        SessionCommand::SendPrompt(cmd) => handle_send_prompt(session, active_run, cmd, now),
        SessionCommand::StopRun(cmd) => handle_stop_run(session, active_run, cmd, now),
        SessionCommand::ResumeSession(cmd) => handle_resume_session(session, active_run, cmd, now),
        SessionCommand::ChangeCwd(cmd) => handle_change_cwd(session, active_run, cmd, now),
        SessionCommand::ArchiveSession(cmd) => {
            handle_archive_session(session, active_run, cmd, now)
        }
        SessionCommand::AnswerQuestion(cmd) => {
            handle_answer_question(session, active_run, cmd, now)
        }
    }
}

fn handle_turn_started(
    session: &Session,
    run: &Run,
    now: Timestamp,
) -> Result<TransitionOutcome, TransitionError> {
    if run.status == RunStatus::CancelRequested {
        return Err(TransitionError::RunCancelling);
    }
    if run.status == RunStatus::WaitingForApproval {
        return Err(TransitionError::RunWaitingForApproval);
    }

    let mut next_session = session.clone();
    let mut next_run = run.clone();
    if run.status == RunStatus::Queued {
        next_session.current_revision = Revision(session.current_revision.0 + 1);
        next_session.updated_at = now;
        next_session.status = SessionStatus::Active;
        next_session.active_run_id = Some(next_run.id.clone());
        next_run.status = RunStatus::Running;
        next_run.revision = Revision(run.revision.0 + 1);
        next_run.updated_at = now;
    }

    Ok(TransitionOutcome {
        new_session: next_session,
        new_run: Some(next_run),
        events: Vec::new(),
    })
}

fn handle_user_question_asked(
    session: &Session,
    run: &Run,
    question_id: &QuestionId,
    prompt: &str,
    options: &[String],
    now: Timestamp,
) -> Result<TransitionOutcome, TransitionError> {
    if run.status == RunStatus::CancelRequested {
        return Err(TransitionError::RunCancelling);
    }
    if run.status == RunStatus::WaitingForApproval {
        return Err(TransitionError::RunWaitingForApproval);
    }

    let mut next_session = session.clone();
    next_session.current_revision = Revision(session.current_revision.0 + 1);
    next_session.updated_at = now;
    next_session.status = SessionStatus::Active;
    next_session.active_run_id = Some(run.id.clone());

    let mut next_run = run.clone();
    next_run.status = RunStatus::WaitingForApproval;
    next_run.revision = Revision(run.revision.0 + 1);
    next_run.updated_at = now;

    Ok(TransitionOutcome {
        new_session: next_session,
        new_run: Some(next_run),
        events: vec![DomainEvent::QuestionAsked {
            question_id: question_id.clone(),
            session_id: session.id.clone(),
            run_id: run.id.clone(),
            prompt: prompt.to_string(),
            options: options.to_vec(),
            timestamp: now,
        }],
    })
}

fn handle_turn_ended(
    session: &Session,
    run: &Run,
    outcome: &TurnEndStatus,
    now: Timestamp,
) -> TransitionOutcome {
    let (final_status, domain_event) = if run.status == RunStatus::CancelRequested {
        (
            RunStatus::Stopped,
            DomainEvent::RunStopped {
                run_id: run.id.clone(),
                timestamp: now,
            },
        )
    } else {
        match outcome {
            TurnEndStatus::Completed => (
                RunStatus::Completed,
                DomainEvent::RunCompleted {
                    run_id: run.id.clone(),
                    timestamp: now,
                },
            ),
            TurnEndStatus::Stopped => (
                RunStatus::Stopped,
                DomainEvent::RunStopped {
                    run_id: run.id.clone(),
                    timestamp: now,
                },
            ),
            TurnEndStatus::Failed { error } => (
                RunStatus::Failed,
                DomainEvent::RunFailed {
                    run_id: run.id.clone(),
                    error: error.clone(),
                    timestamp: now,
                },
            ),
        }
    };

    let mut next_session = session.clone();
    next_session.current_revision = Revision(session.current_revision.0 + 1);
    next_session.updated_at = now;
    if next_session.active_run_id.as_ref() == Some(&run.id) {
        next_session.active_run_id = None;
    }

    let mut next_run = run.clone();
    next_run.status = final_status;
    next_run.revision = Revision(run.revision.0 + 1);
    next_run.updated_at = now;

    TransitionOutcome {
        new_session: next_session,
        new_run: Some(next_run),
        events: vec![domain_event],
    }
}

/// Applies an external DSH runtime lifecycle event to the current session and run state.
///
/// # Errors
/// Returns `TransitionError` if the event targets a mismatched session/run, an archived session,
/// or a state where the event is invalid (such as a terminal run).
pub fn apply_runtime_event(
    session: &Session,
    run: &Run,
    event: &RuntimeEvent,
    now: Timestamp,
) -> Result<TransitionOutcome, TransitionError> {
    if event.session_id() != &session.id {
        return Err(TransitionError::SessionMismatch {
            expected: session.id.clone(),
            actual: event.session_id().clone(),
        });
    }
    if run.session_id != session.id {
        return Err(TransitionError::SessionMismatch {
            expected: session.id.clone(),
            actual: run.session_id.clone(),
        });
    }
    if event.run_id() != &run.id {
        return Err(TransitionError::ActiveRunMismatch {
            active: run.id.clone(),
            requested: event.run_id().clone(),
        });
    }
    if session.status == SessionStatus::Archived {
        return Err(TransitionError::SessionAlreadyArchived);
    }
    if is_terminal_run_status(run.status) {
        return Err(TransitionError::RunAlreadyTerminated);
    }

    match event {
        RuntimeEvent::TurnStarted { .. } => handle_turn_started(session, run, now),
        RuntimeEvent::UserQuestionAsked {
            question_id,
            prompt,
            options,
            ..
        } => handle_user_question_asked(session, run, question_id, prompt, options, now),
        RuntimeEvent::TurnEnded { outcome, .. } => {
            Ok(handle_turn_ended(session, run, outcome, now))
        }
    }
}
