use crate::budget::BudgetPolicy;
use crate::error::TransitionError;
use crate::id::{
    CursorSeq, InviteId, LeaseEpoch, PolicyDigest, ProjectId, Revision, RunId, RunnerId, Timestamp,
    UserId, WorkspaceId,
};
use crate::project::Project;
use crate::state::{Run, RunStatus};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Role-based access control levels inside a shared team workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRole {
    Viewer,
    Member,
    Maintainer,
    Owner,
}

impl WorkspaceRole {
    /// Parses a canonical role string (`owner`, `maintainer`, `member`, `viewer`).
    #[must_use]
    pub fn from_str_name(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "owner" => Some(Self::Owner),
            "maintainer" => Some(Self::Maintainer),
            "member" => Some(Self::Member),
            "viewer" => Some(Self::Viewer),
            _ => None,
        }
    }

    /// Returns the canonical lowercase string name for persistence and HTTP payloads.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Member => "member",
            Self::Maintainer => "maintainer",
            Self::Owner => "owner",
        }
    }

    /// Returns `true` if this role may manage member roles or revoke invitations.
    #[must_use]
    pub const fn can_manage_members(self) -> bool {
        matches!(self, Self::Owner | Self::Maintainer)
    }

    /// Returns `true` if this role may create workspace invitations.
    #[must_use]
    pub const fn can_invite(self) -> bool {
        matches!(self, Self::Owner | Self::Maintainer)
    }

    /// Returns `true` if this role may create/move/plan tasks and post comments.
    #[must_use]
    pub const fn can_mutate_tasks(self) -> bool {
        matches!(self, Self::Owner | Self::Maintainer | Self::Member)
    }

    /// Returns `true` if this role may record human acceptance or change-request decisions.
    #[must_use]
    pub const fn can_accept_decisions(self) -> bool {
        matches!(self, Self::Owner | Self::Maintainer | Self::Member)
    }

    /// Returns `true` if this role may assign runners or force-cancel unreachable runs.
    #[must_use]
    pub const fn can_manage_runners(self) -> bool {
        matches!(self, Self::Owner | Self::Maintainer | Self::Member)
    }
}

impl fmt::Display for WorkspaceRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Workspace actions checked at every command and HTTP/sync boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkspaceAction {
    ReadWorkspace,
    MutateTasks,
    RecordDecision,
    ManageRunners,
    InviteMembers,
    ManageMembers,
    PromoteProject,
}

impl WorkspaceAction {
    const fn label(self) -> &'static str {
        match self {
            Self::ReadWorkspace => "read_workspace",
            Self::MutateTasks => "mutate_tasks",
            Self::RecordDecision => "record_decision",
            Self::ManageRunners => "manage_runners",
            Self::InviteMembers => "invite_members",
            Self::ManageMembers => "manage_members",
            Self::PromoteProject => "promote_project",
        }
    }
}

/// Pure authorization check verifying `role` is currently permitted to perform `action`.
///
/// # Errors
/// Returns [`TransitionError::WorkspacePermissionDenied`] if `role` lacks capability for `action`.
pub fn authorize_workspace_action(
    role: WorkspaceRole,
    action: WorkspaceAction,
) -> Result<(), TransitionError> {
    let allowed = match action {
        WorkspaceAction::ReadWorkspace => true,
        WorkspaceAction::MutateTasks => role.can_mutate_tasks(),
        WorkspaceAction::RecordDecision => role.can_accept_decisions(),
        WorkspaceAction::ManageRunners => role.can_manage_runners(),
        WorkspaceAction::InviteMembers => role.can_invite(),
        WorkspaceAction::ManageMembers | WorkspaceAction::PromoteProject => {
            role.can_manage_members()
        }
    };
    if !allowed {
        return Err(TransitionError::WorkspacePermissionDenied {
            action: action.label().to_string(),
            role: role.as_str().to_string(),
        });
    }
    Ok(())
}

/// Shared collaboration workspace record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub slug: String,
    pub created_by: UserId,
    pub revision: Revision,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Active participant membership within a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMember {
    pub workspace_id: WorkspaceId,
    pub user_id: UserId,
    pub display_name: String,
    pub role: WorkspaceRole,
    pub joined_at: Timestamp,
}

/// Lifecycle state of a workspace invitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvitationStatus {
    Pending,
    Accepted,
    Revoked,
    Expired,
}

/// Workspace invitation record (secret token is never persisted in plaintext).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceInvitation {
    pub id: InviteId,
    pub workspace_id: WorkspaceId,
    pub invitee_hint: String,
    pub role: WorkspaceRole,
    pub status: InvitationStatus,
    pub invited_by: UserId,
    pub expires_at: Timestamp,
    pub created_at: Timestamp,
    pub accepted_at: Option<Timestamp>,
    pub accepted_by: Option<UserId>,
}

/// Ownership mode distinguishing solo-local projects from server-coordinated team projects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ProjectOwnershipMode {
    #[default]
    SoloLocal,
    TeamManaged {
        workspace_id: WorkspaceId,
        server_url: String,
        ownership_epoch: u64,
    },
}

/// Sanitized shared project specification uploaded to `runlark-server`.
///
/// Intentionally omits local checkout paths (`repo_root`) and developer credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamProjectSpec {
    pub project_id: ProjectId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub default_branch: String,
    pub max_concurrent_runs: u32,
    pub budget_policy: BudgetPolicy,
    pub policy_digest: PolicyDigest,
    pub ownership_epoch: u64,
    pub revision: Revision,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Extracts a sanitized shared project payload without leaking local checkout paths.
#[must_use]
pub fn sanitize_project_for_team_snapshot(
    project: &Project,
    workspace_id: WorkspaceId,
    ownership_epoch: u64,
) -> TeamProjectSpec {
    TeamProjectSpec {
        project_id: project.id.clone(),
        workspace_id,
        name: project.name.clone(),
        default_branch: project.default_branch.clone(),
        max_concurrent_runs: project.max_concurrent_runs,
        budget_policy: project.budget_policy.clone(),
        policy_digest: project.policy_digest.clone(),
        ownership_epoch,
        revision: project.revision,
        created_at: project.created_at,
        updated_at: project.updated_at,
    }
}

/// Explicit server acknowledgement required to transition a solo project into team ownership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerPromotionAck {
    pub workspace_id: WorkspaceId,
    pub project_id: ProjectId,
    pub initial_cursor: CursorSeq,
    pub ownership_epoch: u64,
    pub acknowledged_at: Timestamp,
}

/// Verifies that a local solo project has zero active, queued, waiting, or interrupted runs
/// before promoting to team mode.
///
/// # Errors
/// Returns [`TransitionError::SoloToTeamBlockedByActiveRuns`] if any run is non-terminal.
pub fn validate_solo_to_team_promotion(runs: &[Run]) -> Result<(), TransitionError> {
    let blocking = runs.iter().filter(|run| !run.status.is_terminal()).count();
    if blocking > 0 {
        return Err(TransitionError::SoloToTeamBlockedByActiveRuns {
            active_runs: blocking,
        });
    }
    Ok(())
}

/// Liveness status of a registered execution runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerStatus {
    Online,
    Unreachable,
    Drained,
}

impl RunnerStatus {
    /// Returns the canonical lowercase string label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Unreachable => "unreachable",
            Self::Drained => "drained",
        }
    }

    /// Parses a runner status string.
    #[must_use]
    pub fn from_str_name(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "online" => Some(Self::Online),
            "unreachable" => Some(Self::Unreachable),
            "drained" => Some(Self::Drained),
            _ => None,
        }
    }
}

/// Registered execution runner owned by a workspace participant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerRegistration {
    pub id: RunnerId,
    pub workspace_id: WorkspaceId,
    pub owner_user_id: UserId,
    pub name: String,
    pub status: RunnerStatus,
    pub last_heartbeat_at: Timestamp,
    pub created_at: Timestamp,
}

/// Exclusive ownership and fencing tuple bound to a server-coordinated run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RunOwnership {
    pub assigned_runner_id: Option<RunnerId>,
    pub lease_epoch: LeaseEpoch,
    pub runner_unreachable: bool,
}

/// Enforces that only an unassigned non-terminal run can receive a runner assignment, and only an
/// `Online` runner can be assigned. Active runs owned by another runner cannot be taken over.
///
/// # Errors
/// Returns [`TransitionError`] if the run is terminal, already owned by another runner, or if
/// the target runner is not `Online`.
pub fn validate_runner_assignment(
    run_id: &RunId,
    run_status: RunStatus,
    ownership: &RunOwnership,
    target_runner: &RunnerRegistration,
) -> Result<LeaseEpoch, TransitionError> {
    if run_status.is_terminal() {
        return Err(TransitionError::RunAlreadyTerminated);
    }
    if target_runner.status != RunnerStatus::Online {
        return Err(TransitionError::RunnerNotOnline {
            runner_id: target_runner.id.clone(),
            status: target_runner.status.as_str().to_string(),
        });
    }
    if let Some(current_owner) = &ownership.assigned_runner_id
        && current_owner != &target_runner.id
    {
        return Err(TransitionError::ActiveRunPreventsTakeover {
            run_id: run_id.clone(),
            owner_runner_id: current_owner.clone(),
        });
    }
    if ownership.assigned_runner_id.as_ref() == Some(&target_runner.id)
        && ownership.lease_epoch.0 > 0
    {
        return Ok(ownership.lease_epoch);
    }
    Ok(LeaseEpoch(ownership.lease_epoch.0.saturating_add(1)))
}

/// Verifies writer-side fencing `(runner_id, lease_epoch)` before accepting run mutations/events.
///
/// # Errors
/// Returns [`TransitionError::RunnerFencingMismatch`] if the writer does not hold the current lease.
pub fn validate_runner_fencing(
    run_id: &RunId,
    ownership: &RunOwnership,
    actor_runner_id: &RunnerId,
    presented_epoch: LeaseEpoch,
) -> Result<(), TransitionError> {
    let matches_owner = ownership
        .assigned_runner_id
        .as_ref()
        .is_some_and(|owner| owner == actor_runner_id);
    if !matches_owner || ownership.lease_epoch != presented_epoch || presented_epoch.0 == 0 {
        return Err(TransitionError::RunnerFencingMismatch {
            run_id: run_id.clone(),
            expected_runner: ownership.assigned_runner_id.clone(),
            expected_epoch: ownership.lease_epoch,
            actual_runner: actor_runner_id.clone(),
            actual_epoch: presented_epoch,
        });
    }
    Ok(())
}
