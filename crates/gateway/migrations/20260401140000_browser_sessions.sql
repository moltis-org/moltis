-- Browser session history and action log.
-- Persists session metadata so closed sessions remain visible
-- in the browser management UI for debugging and review.

CREATE TABLE IF NOT EXISTS browser_sessions (
    session_id    TEXT    PRIMARY KEY,
    sandboxed     INTEGER NOT NULL DEFAULT 0,
    url           TEXT,
    created_at    TEXT    NOT NULL DEFAULT (datetime('now')),
    closed_at     TEXT
);

CREATE INDEX IF NOT EXISTS idx_browser_sessions_created
    ON browser_sessions(created_at DESC);

CREATE TABLE IF NOT EXISTS browser_action_log (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id    TEXT    NOT NULL,
    action        TEXT    NOT NULL,
    url           TEXT,
    success       INTEGER NOT NULL DEFAULT 1,
    error         TEXT,
    duration_ms   INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT    NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (session_id) REFERENCES browser_sessions(session_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_browser_action_log_session
    ON browser_action_log(session_id, created_at DESC);
