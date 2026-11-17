-- 0002_questions.sql: Pending human questions and approval requests
CREATE TABLE IF NOT EXISTS questions (
    question_id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    prompt TEXT NOT NULL,
    options_json TEXT NOT NULL,
    status TEXT NOT NULL,
    answer_json TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_questions_status ON questions(status);
CREATE INDEX IF NOT EXISTS idx_questions_run ON questions(run_id);
CREATE INDEX IF NOT EXISTS idx_questions_session ON questions(session_id);
