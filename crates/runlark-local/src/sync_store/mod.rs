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

/// Atomic `SQLite` synchronization, promotion, and runner ownership store.
pub struct SyncStore;
