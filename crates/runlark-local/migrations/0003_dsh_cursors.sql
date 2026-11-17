CREATE TABLE IF NOT EXISTS dsh_cursors (
    session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
    last_seq INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL
);
