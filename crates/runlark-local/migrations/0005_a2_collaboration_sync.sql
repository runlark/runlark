-- 0005_a2_collaboration_sync.sql: A2 team bindings, sync cursors, comments, offline drafts, outbox, and runner ownership
CREATE TABLE IF NOT EXISTS project_sync_bindings (
    project_id TEXT PRIMARY KEY NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
    ownership_mode TEXT NOT NULL,
    workspace_id TEXT,
    server_url TEXT,
    ownership_epoch INTEGER NOT NULL DEFAULT 0,
    last_cursor_seq INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_project_sync_workspace ON project_sync_bindings(workspace_id);

CREATE TABLE IF NOT EXISTS task_comments (
    comment_id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL REFERENCES tasks(task_id) ON DELETE CASCADE,
    run_id TEXT REFERENCES runs(run_id) ON DELETE SET NULL,
    author_user_id TEXT NOT NULL,
    author_display_name TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_task_comments_task ON task_comments(task_id, created_at);

CREATE TABLE IF NOT EXISTS offline_drafts (
    draft_id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
    task_id TEXT REFERENCES tasks(task_id) ON DELETE CASCADE,
    draft_kind TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    base_revision INTEGER,
    sync_state TEXT NOT NULL,
    conflict_server_revision INTEGER,
    conflict_reason TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_offline_drafts_project ON offline_drafts(project_id, sync_state);
CREATE INDEX IF NOT EXISTS idx_offline_drafts_task ON offline_drafts(task_id, created_at);

CREATE TABLE IF NOT EXISTS sync_outbox (
    command_id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL,
    project_id TEXT,
    draft_id TEXT REFERENCES offline_drafts(draft_id) ON DELETE SET NULL,
    command_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    expected_revision INTEGER,
    status TEXT NOT NULL,
    last_error TEXT,
    conflict_server_revision INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sync_outbox_status ON sync_outbox(workspace_id, status, created_at);

CREATE TABLE IF NOT EXISTS run_ownership (
    run_id TEXT PRIMARY KEY NOT NULL REFERENCES runs(run_id) ON DELETE CASCADE,
    assigned_runner_id TEXT,
    lease_epoch INTEGER NOT NULL DEFAULT 0,
    runner_unreachable INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_run_ownership_runner ON run_ownership(assigned_runner_id, lease_epoch);

CREATE TABLE IF NOT EXISTS team_workspaces (
    workspace_id TEXT PRIMARY KEY NOT NULL,
    server_url TEXT NOT NULL,
    name TEXT NOT NULL,
    slug TEXT NOT NULL,
    created_by TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1,
    last_cursor_seq INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS team_members (
    workspace_id TEXT NOT NULL REFERENCES team_workspaces(workspace_id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    role TEXT NOT NULL,
    joined_at INTEGER NOT NULL,
    PRIMARY KEY (workspace_id, user_id)
);

CREATE TABLE IF NOT EXISTS team_runners (
    runner_id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES team_workspaces(workspace_id) ON DELETE CASCADE,
    owner_user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    last_heartbeat_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_team_runners_workspace ON team_runners(workspace_id);

CREATE TABLE IF NOT EXISTS team_auth_sessions (
    server_url TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    bearer_token TEXT NOT NULL,
    default_workspace_id TEXT,
    runner_id TEXT,
    runner_name TEXT,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS runner_buffered_events (
    run_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    workspace_id TEXT NOT NULL,
    runner_id TEXT NOT NULL,
    lease_epoch INTEGER NOT NULL,
    status TEXT,
    workflow_stage TEXT,
    head_sha TEXT,
    base_sha TEXT,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    flushed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (run_id, seq)
);

CREATE INDEX IF NOT EXISTS idx_runner_buffered_unflushed ON runner_buffered_events(flushed, run_id, seq);

CREATE TABLE IF NOT EXISTS synced_cas_blobs (
    workspace_id TEXT NOT NULL,
    digest_sha256 TEXT NOT NULL,
    uploaded_at INTEGER NOT NULL,
    PRIMARY KEY (workspace_id, digest_sha256)
);
