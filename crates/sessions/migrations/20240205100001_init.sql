-- Sessions table schema
-- Owned by: moltis-sessions crate
-- Depends on: moltis-projects (sessions.project_id references projects.id)

CREATE TABLE IF NOT EXISTS sessions (
    key             TEXT    PRIMARY KEY,
    id              TEXT    NOT NULL,
    label           TEXT,
    model           TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    message_count   INTEGER NOT NULL DEFAULT 0,
    project_id      TEXT    REFERENCES projects(id) ON DELETE SET NULL,
    archived        INTEGER NOT NULL DEFAULT 0,
    worktree_branch TEXT,
    sandbox_enabled INTEGER,
    sandbox_image   TEXT,
    channel_binding TEXT
);

CREATE INDEX IF NOT EXISTS idx_sessions_created_at ON sessions(created_at);

CREATE TABLE IF NOT EXISTS channel_sessions (
    channel_type TEXT    NOT NULL,
    account_id   TEXT    NOT NULL,
    chat_id      TEXT    NOT NULL,
    session_key  TEXT    NOT NULL,
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY (channel_type, account_id, chat_id)
);
