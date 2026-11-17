use runlark_domain::{
    ArtifactDigest, CommandId, DecisionId, DomainError, ProjectId, QuestionId, RunId, SessionId,
    TaskId, TransitionError,
};
use thiserror::Error;

/// Storage and persistence errors for local `SQLite` and CAS operations.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Filesystem I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Domain transition error: {0}")]
    Transition(#[from] TransitionError),

    #[error("Domain validation error: {0}")]
    Domain(#[from] DomainError),

    #[error("Command payload conflict: command {0} exists with differing payload")]
    CommandPayloadConflict(CommandId),

    #[error("Session not found: {0}")]
    SessionNotFound(SessionId),

    #[error("Run not found: {0}")]
    RunNotFound(RunId),

    #[error("Question not found: {0}")]
    QuestionNotFound(QuestionId),

    #[error("Project not found: {0}")]
    ProjectNotFound(ProjectId),

    #[error("Task not found: {0}")]
    TaskNotFound(TaskId),

    #[error("Decision not found: {0}")]
    DecisionNotFound(DecisionId),

    #[error("Artifact blob not found in CAS: {0}")]
    ArtifactNotFound(ArtifactDigest),

    #[error("Corrupt CAS artifact {digest}: expected {expected}, got {actual}")]
    CorruptArtifact {
        digest: ArtifactDigest,
        expected: String,
        actual: String,
    },

    #[error("Invalid storage state: {0}")]
    InvalidState(String),
}
