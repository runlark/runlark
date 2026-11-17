-- 0004_a1_board_and_history.sql: A1 projects, tasks, attempts, decisions, CAS artifacts, and run metadata
CREATE TABLE IF NOT EXISTS projects (
    project_id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    repo_root TEXT NOT NULL,
    default_branch TEXT NOT NULL,
    max_concurrent_runs INTEGER NOT NULL DEFAULT 2,
    budget_policy_json TEXT NOT NULL,
    policy_digest TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS tasks (
    task_id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(project_id),
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
    revision INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tasks_project_stage ON tasks(project_id, stage);
CREATE INDEX IF NOT EXISTS idx_tasks_stage ON tasks(stage);

ALTER TABLE runs ADD COLUMN task_id TEXT;
ALTER TABLE runs ADD COLUMN project_id TEXT;
ALTER TABLE runs ADD COLUMN workflow_stage TEXT NOT NULL DEFAULT 'implementation';
ALTER TABLE runs ADD COLUMN task_kind TEXT NOT NULL DEFAULT 'standard';
ALTER TABLE runs ADD COLUMN workflow_digest TEXT;
ALTER TABLE runs ADD COLUMN base_sha TEXT;
ALTER TABLE runs ADD COLUMN head_sha TEXT;
ALTER TABLE runs ADD COLUMN plan_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE runs ADD COLUMN policy_digest TEXT;
ALTER TABLE runs ADD COLUMN budget_json TEXT;
ALTER TABLE runs ADD COLUMN cost_summary_json TEXT;
ALTER TABLE runs ADD COLUMN repair_attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN pr_url TEXT;

CREATE INDEX IF NOT EXISTS idx_runs_status_created ON runs(status, created_at);
CREATE INDEX IF NOT EXISTS idx_runs_task ON runs(task_id);

CREATE TABLE IF NOT EXISTS attempts (
    attempt_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    attempt_no INTEGER NOT NULL,
    workflow_stage TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    outcome_json TEXT NOT NULL,
    cost_micro_usd INTEGER,
    unknown_cost_reason TEXT,
    repair_reason TEXT,
    UNIQUE(run_id, attempt_no)
);

CREATE INDEX IF NOT EXISTS idx_attempts_run ON attempts(run_id, attempt_no);

CREATE TABLE IF NOT EXISTS decisions (
    decision_id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL REFERENCES tasks(task_id),
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    kind TEXT NOT NULL,
    actor TEXT NOT NULL,
    plan_version INTEGER NOT NULL,
    head_sha TEXT NOT NULL,
    base_sha TEXT NOT NULL,
    policy_digest TEXT NOT NULL,
    evidence_digest TEXT NOT NULL,
    comment TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_decisions_task ON decisions(task_id, created_at);
CREATE INDEX IF NOT EXISTS idx_decisions_run ON decisions(run_id, created_at);

CREATE TABLE IF NOT EXISTS artifacts (
    digest_sha256 TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL,
    media_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS run_artifacts (
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    digest_sha256 TEXT NOT NULL REFERENCES artifacts(digest_sha256),
    label TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    PRIMARY KEY (run_id, digest_sha256, label)
);

CREATE TABLE IF NOT EXISTS board_commands (
    command_id TEXT PRIMARY KEY NOT NULL,
    aggregate_type TEXT NOT NULL,
    aggregate_id TEXT NOT NULL,
    command_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    expected_revision INTEGER NOT NULL,
    status TEXT NOT NULL,
    error_message TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_board_commands_aggregate ON board_commands(aggregate_type, aggregate_id);
