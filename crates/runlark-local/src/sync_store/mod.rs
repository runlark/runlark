mod apply;
mod queries;
mod upsert;

use runlark_domain::{
    CursorSeq, Decision, Project, ProjectId, ProjectOwnershipMode, Run, Task, Timestamp, Workspace,
};
use serde::{Deserialize, Serialize};

/// Sanitized local project snapshot bundle collected when promoting `SoloLocal` -> `TeamManaged`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionBundle {
    pub project: Project,
    pub tasks: Vec<Task>,
    pub runs: Vec<Run>,
    pub decisions: Vec<Decision>,
}

/// Local persistence record binding a project to either `SoloLocal` or `TeamManaged` mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSyncBinding {
    pub project_id: ProjectId,
    pub ownership_mode: ProjectOwnershipMode,
    pub last_cursor_seq: CursorSeq,
    pub updated_at: Timestamp,
}

/// Cached team workspace metadata and current sync cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamWorkspaceRecord {
    pub workspace: Workspace,
    pub server_url: String,
    pub last_cursor_seq: CursorSeq,
}

/// SQL predicate over `runs r` matching runs executed by this daemon (`SoloLocal`, legacy A0, or
/// `TeamManaged` runs leased to this daemon's registered runner) while excluding remote teammates'
/// synced `TeamManaged` runs.
pub(crate) const LOCAL_OWNED_RUN_PREDICATE: &str = concat!(
    "(",
    "NOT EXISTS (",
    "SELECT 1 FROM project_sync_bindings psb ",
    "WHERE psb.ownership_mode = 'team_managed' ",
    "AND (psb.project_id = r.project_id ",
    "OR psb.project_id = (SELECT t.project_id FROM tasks t WHERE t.task_id = r.task_id))",
    ") ",
    "OR EXISTS (",
    "SELECT 1 FROM project_sync_bindings psb ",
    "INNER JOIN run_ownership ro ON ro.run_id = r.run_id ",
    "INNER JOIN team_auth_sessions tas ON tas.server_url = psb.server_url ",
    "WHERE psb.ownership_mode = 'team_managed' ",
    "AND (psb.project_id = r.project_id ",
    "OR psb.project_id = (SELECT t.project_id FROM tasks t WHERE t.task_id = r.task_id)) ",
    "AND ro.assigned_runner_id IS NOT NULL ",
    "AND ro.assigned_runner_id = tas.runner_id ",
    "AND ro.lease_epoch > 0 ",
    "AND ro.runner_unreachable = 0",
    ")",
    ")"
);

/// Atomic `SQLite` synchronization, promotion, and runner ownership store.
pub struct SyncStore;
