use crate::id::{CommandId, EventSeq, Revision, RunId, SessionId, Timestamp};
use serde::{Deserialize, Serialize};

/// Command to send a user prompt to a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendPromptCommand {
    pub command_id: CommandId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub prompt: String,
    pub expected_revision: Revision,
    pub created_at: Timestamp,
}

/// Command to stop an active run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopRunCommand {
    pub command_id: CommandId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub reason: Option<String>,
    pub expected_revision: Revision,
    pub created_at: Timestamp,
}

/// Command to resume an interrupted or disconnected session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeSessionCommand {
    pub command_id: CommandId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub after_seq: Option<EventSeq>,
    pub expected_revision: Revision,
    pub created_at: Timestamp,
}

/// Command to change the working directory of a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeCwdCommand {
    pub command_id: CommandId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub new_cwd: String,
    pub expected_revision: Revision,
    pub created_at: Timestamp,
}

/// Command to archive a completed session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveSessionCommand {
    pub command_id: CommandId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub expected_revision: Revision,
    pub created_at: Timestamp,
}

/// Enum of all managed session commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum SessionCommand {
    SendPrompt(SendPromptCommand),
    StopRun(StopRunCommand),
    ResumeSession(ResumeSessionCommand),
    ChangeCwd(ChangeCwdCommand),
    ArchiveSession(ArchiveSessionCommand),
}

impl SessionCommand {
    #[must_use]
    pub fn command_id(&self) -> &CommandId {
        match self {
            Self::SendPrompt(c) => &c.command_id,
            Self::StopRun(c) => &c.command_id,
            Self::ResumeSession(c) => &c.command_id,
            Self::ChangeCwd(c) => &c.command_id,
            Self::ArchiveSession(c) => &c.command_id,
        }
    }

    #[must_use]
    pub fn session_id(&self) -> &SessionId {
        match self {
            Self::SendPrompt(c) => &c.session_id,
            Self::StopRun(c) => &c.session_id,
            Self::ResumeSession(c) => &c.session_id,
            Self::ChangeCwd(c) => &c.session_id,
            Self::ArchiveSession(c) => &c.session_id,
        }
    }

    #[must_use]
    pub fn run_id(&self) -> &RunId {
        match self {
            Self::SendPrompt(c) => &c.run_id,
            Self::StopRun(c) => &c.run_id,
            Self::ResumeSession(c) => &c.run_id,
            Self::ChangeCwd(c) => &c.run_id,
            Self::ArchiveSession(c) => &c.run_id,
        }
    }

    #[must_use]
    pub fn expected_revision(&self) -> Revision {
        match self {
            Self::SendPrompt(c) => c.expected_revision,
            Self::StopRun(c) => c.expected_revision,
            Self::ResumeSession(c) => c.expected_revision,
            Self::ChangeCwd(c) => c.expected_revision,
            Self::ArchiveSession(c) => c.expected_revision,
        }
    }
}
