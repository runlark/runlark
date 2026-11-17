use crate::command::{
    ArchiveSessionCommand, ChangeCwdCommand, ResumeSessionCommand, SendPromptCommand,
    SessionCommand, StopRunCommand,
};
use crate::error::TransitionError;
use crate::event::DomainEvent;
use crate::id::{Revision, Timestamp};
use crate::state::{Run, RunStatus, Session, SessionStatus};

/// Result of applying a domain command to a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionOutcome {
    pub new_session: Session,
    pub new_run: Option<Run>,
    pub events: Vec<DomainEvent>,
}

fn is_terminal_run_status(status: RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Stopped | RunStatus::Completed | RunStatus::Failed
    )
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

    let mut run = active_run
        .filter(|r| !is_terminal_run_status(r.status))
        .cloned()
        .unwrap_or_else(|| Run {
            id: cmd.run_id.clone(),
            session_id: session.id.clone(),
            status: RunStatus::Queued,
            worktree_path: session.cwd.clone(),
            revision: Revision(1),
            created_at: now,
            updated_at: now,
        });
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
    let Some(run) = active_run else {
        return Err(TransitionError::NoActiveRun);
    };
    if is_terminal_run_status(run.status) {
        return Err(TransitionError::RunAlreadyTerminated);
    }

    let mut next_session = session.clone();
    next_session.current_revision = Revision(session.current_revision.0 + 1);
    next_session.updated_at = now;

    let mut next_run = run.clone();
    // INVARIANT: Stop does not transition directly to Stopped!
    next_run.status = RunStatus::CancelRequested;
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
) -> TransitionOutcome {
    let mut next_session = session.clone();
    next_session.current_revision = Revision(session.current_revision.0 + 1);
    next_session.updated_at = now;

    let mut next_run = active_run.cloned();
    if let Some(ref mut r) = next_run
        && r.status == RunStatus::Interrupted
    {
        r.status = RunStatus::Running;
        r.updated_at = now;
    }

    let events = next_run
        .as_ref()
        .map(|run| DomainEvent::SessionResumed {
            session_id: session.id.clone(),
            run_id: run.id.clone(),
            command_id: cmd.command_id.clone(),
            after_seq: cmd.after_seq,
            timestamp: now,
        })
        .into_iter()
        .collect();

    TransitionOutcome {
        new_session: next_session,
        new_run: next_run,
        events,
    }
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
        SessionCommand::ResumeSession(cmd) => {
            Ok(handle_resume_session(session, active_run, cmd, now))
        }
        SessionCommand::ChangeCwd(cmd) => handle_change_cwd(session, active_run, cmd, now),
        SessionCommand::ArchiveSession(cmd) => {
            handle_archive_session(session, active_run, cmd, now)
        }
    }
}
