-- Projects table schema
-- Owned by: moltis-projects crate

CREATE TABLE IF NOT EXISTS projects (
    id               TEXT    PRIMARY KEY,
    label            TEXT    NOT NULL,
    directory        TEXT    NOT NULL,
    system_prompt    TEXT,
    auto_worktree    INTEGER NOT NULL DEFAULT 0,
    setup_command    TEXT,
    teardown_command TEXT,
    branch_prefix    TEXT,
    sandbox_image    TEXT,
    detected         INTEGER NOT NULL DEFAULT 0,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_projects_updated_at ON projects(updated_at);
