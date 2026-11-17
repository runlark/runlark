use crate::error::DomainError;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

macro_rules! define_string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

define_string_id!(
    /// Unique identifier for an agent session.
    SessionId
);
define_string_id!(
    /// Unique identifier for a run execution.
    RunId
);
define_string_id!(
    /// Unique identifier for a task specification.
    TaskId
);
define_string_id!(
    /// Unique identifier for a persisted command.
    CommandId
);
define_string_id!(
    /// Unique identifier for a human question or approval request.
    QuestionId
);
define_string_id!(
    /// Unique identifier for a project workspace.
    ProjectId
);
define_string_id!(
    /// Unique identifier for a run attempt.
    AttemptId
);
define_string_id!(
    /// Unique identifier for a human decision record.
    DecisionId
);
define_string_id!(
    /// Unique identifier for a workflow step.
    StepId
);
define_string_id!(
    /// Unique identifier for a workflow/validator plugin package.
    PluginId
);
define_string_id!(
    /// SHA-256 digest or version identifier binding a project's budget/review policy.
    PolicyDigest
);

/// Validated 64-character lowercase hexadecimal SHA-256 digest for CAS artifacts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct ArtifactDigest(String);

impl ArtifactDigest {
    /// Parses and validates a 64-character lowercase hexadecimal SHA-256 digest.
    ///
    /// # Errors
    /// Returns [`DomainError::InvalidDigest`] if `value` is not 64 lowercase hex characters.
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainError> {
        let raw = value.into();
        if raw.len() != 64
            || !raw
                .chars()
                .all(|ch| ch.is_ascii_digit() || ('a'..='f').contains(&ch))
        {
            return Err(DomainError::InvalidDigest(raw));
        }
        Ok(Self(raw))
    }

    /// Returns the underlying 64-character lowercase hex digest string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ArtifactDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for ArtifactDigest {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for ArtifactDigest {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl TryFrom<String> for ArtifactDigest {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for ArtifactDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(raw).map_err(serde::de::Error::custom)
    }
}

/// Optimistic concurrency control revision number.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct Revision(pub u64);

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Monotonically increasing sequence number for events in a run.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct EventSeq(pub u64);

impl fmt::Display for EventSeq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Wall-clock timestamp in milliseconds since UNIX epoch.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct Timestamp(pub u64);

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
