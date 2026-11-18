-- 0001_initial_team.sql: PostgreSQL schema for Runlark SaaS/self-host collaboration

CREATE TABLE IF NOT EXISTS users (
    user_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    email TEXT,
    created_at BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS auth_tokens (
    token_hash TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(user_id),
    label TEXT NOT NULL,
    expires_at BIGINT,
    revoked_at BIGINT,
    created_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_auth_tokens_user ON auth_tokens(user_id);

CREATE TABLE IF NOT EXISTS workspaces (
    workspace_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    created_by TEXT NOT NULL REFERENCES users(user_id),
    revision BIGINT NOT NULL DEFAULT 1,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS workspace_members (
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    user_id TEXT NOT NULL REFERENCES users(user_id),
    role TEXT NOT NULL,
    joined_at BIGINT NOT NULL,
    PRIMARY KEY (workspace_id, user_id)
);

CREATE TABLE IF NOT EXISTS workspace_invitations (
    invite_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    invitee_hint TEXT NOT NULL,
    role TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL,
    invited_by TEXT NOT NULL REFERENCES users(user_id),
    expires_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL,
    accepted_at BIGINT,
    accepted_by TEXT REFERENCES users(user_id)
);

CREATE INDEX IF NOT EXISTS idx_invitations_workspace ON workspace_invitations(workspace_id, status);

CREATE TABLE IF NOT EXISTS runners (
    runner_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    owner_user_id TEXT NOT NULL REFERENCES users(user_id),
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    last_heartbeat_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_runners_workspace ON runners(workspace_id, status);

CREATE TABLE IF NOT EXISTS team_projects (
    project_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    name TEXT NOT NULL,
    default_branch TEXT NOT NULL,
    max_concurrent_runs INTEGER NOT NULL DEFAULT 2,
    budget_policy_json TEXT NOT NULL,
    policy_digest TEXT NOT NULL,
    ownership_epoch BIGINT NOT NULL DEFAULT 1,
    revision BIGINT NOT NULL DEFAULT 1,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_team_projects_workspace ON team_projects(workspace_id);

CREATE TABLE IF NOT EXISTS team_tasks (
    task_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    project_id TEXT NOT NULL REFERENCES team_projects(project_id),
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    kind TEXT NOT NULL,
    stage TEXT NOT NULL,
    workflow_stage TEXT NOT NULL,
    plan_text TEXT,
    plan_version INTEGER NOT NULL DEFAULT 1,
    base_ref TEXT,
    head_ref TEXT,
    pr_url TEXT,
    active_session_id TEXT,
    active_run_id TEXT,
    latest_run_id TEXT,
    budget_override_json TEXT,
    revision BIGINT NOT NULL DEFAULT 1,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_team_tasks_workspace ON team_tasks(workspace_id, project_id, stage);

CREATE TABLE IF NOT EXISTS team_runs (
    run_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    project_id TEXT NOT NULL REFERENCES team_projects(project_id),
    task_id TEXT NOT NULL REFERENCES team_tasks(task_id),
    session_id TEXT NOT NULL,
    status TEXT NOT NULL,
    workflow_stage TEXT NOT NULL,
    task_kind TEXT NOT NULL,
    workflow_digest TEXT,
    base_sha TEXT,
    head_sha TEXT,
    plan_version INTEGER NOT NULL DEFAULT 1,
    policy_digest TEXT,
    budget_json TEXT NOT NULL,
    cost_summary_json TEXT NOT NULL,
    repair_attempts INTEGER NOT NULL DEFAULT 0,
    pr_url TEXT,
    assigned_runner_id TEXT REFERENCES runners(runner_id),
    lease_epoch BIGINT NOT NULL DEFAULT 0,
    runner_unreachable BOOLEAN NOT NULL DEFAULT FALSE,
    revision BIGINT NOT NULL DEFAULT 1,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_team_runs_workspace ON team_runs(workspace_id, task_id);

CREATE TABLE IF NOT EXISTS team_attempts (
    attempt_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    run_id TEXT NOT NULL REFERENCES team_runs(run_id),
    attempt_no INTEGER NOT NULL,
    workflow_stage TEXT NOT NULL,
    started_at BIGINT NOT NULL,
    ended_at BIGINT,
    outcome_json TEXT NOT NULL,
    cost_micro_usd BIGINT,
    unknown_cost_reason TEXT,
    repair_reason TEXT,
    UNIQUE (run_id, attempt_no)
);

CREATE INDEX IF NOT EXISTS idx_team_attempts_workspace ON team_attempts(workspace_id, started_at, attempt_no);

CREATE TABLE IF NOT EXISTS team_decisions (
    decision_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    task_id TEXT NOT NULL REFERENCES team_tasks(task_id),
    run_id TEXT NOT NULL REFERENCES team_runs(run_id),
    kind TEXT NOT NULL,
    actor TEXT NOT NULL,
    plan_version INTEGER NOT NULL,
    head_sha TEXT NOT NULL,
    base_sha TEXT NOT NULL,
    policy_digest TEXT NOT NULL,
    evidence_digest TEXT NOT NULL,
    comment TEXT,
    created_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_team_decisions_task ON team_decisions(task_id, created_at);

CREATE TABLE IF NOT EXISTS team_comments (
    comment_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    task_id TEXT NOT NULL REFERENCES team_tasks(task_id),
    run_id TEXT REFERENCES team_runs(run_id),
    author_user_id TEXT NOT NULL REFERENCES users(user_id),
    author_display_name TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_team_comments_task ON team_comments(task_id, created_at);

CREATE TABLE IF NOT EXISTS team_artifacts (
    digest_sha256 TEXT PRIMARY KEY,
    size_bytes BIGINT NOT NULL,
    created_at BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS team_workspace_artifacts (
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    digest_sha256 TEXT NOT NULL REFERENCES team_artifacts(digest_sha256),
    kind TEXT NOT NULL,
    media_type TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (workspace_id, digest_sha256)
);

CREATE TABLE IF NOT EXISTS team_run_artifacts (
    run_id TEXT NOT NULL REFERENCES team_runs(run_id),
    digest_sha256 TEXT NOT NULL REFERENCES team_artifacts(digest_sha256),
    label TEXT NOT NULL DEFAULT '',
    created_at BIGINT NOT NULL,
    PRIMARY KEY (run_id, digest_sha256, label)
);

CREATE TABLE IF NOT EXISTS team_commands (
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    command_id TEXT NOT NULL,
    command_type TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    response_json TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (workspace_id, command_id)
);

CREATE TABLE IF NOT EXISTS workspace_events (
    cursor_seq BIGSERIAL PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    project_id TEXT,
    aggregate_type TEXT NOT NULL,
    aggregate_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    actor_user_id TEXT NOT NULL,
    created_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_workspace_events_ws_cursor ON workspace_events(workspace_id, cursor_seq);

CREATE TABLE IF NOT EXISTS team_run_events (
    run_id TEXT NOT NULL REFERENCES team_runs(run_id),
    run_seq BIGINT NOT NULL,
    runner_id TEXT NOT NULL REFERENCES runners(runner_id),
    lease_epoch BIGINT NOT NULL,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (run_id, run_seq)
);
