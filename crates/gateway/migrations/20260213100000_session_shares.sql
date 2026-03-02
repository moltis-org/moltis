-- Immutable session-share snapshots for public/private sharing.
CREATE TABLE IF NOT EXISTS session_shares (
    id                     TEXT    PRIMARY KEY,
    session_key            TEXT    NOT NULL,
    visibility             TEXT    NOT NULL CHECK (visibility IN ('public', 'private')),
    snapshot_json          TEXT    NOT NULL,
    snapshot_message_count INTEGER NOT NULL,
    token_hash             TEXT,
    views                  INTEGER NOT NULL DEFAULT 0,
    created_at             INTEGER NOT NULL,
    revoked_at             INTEGER
);

CREATE INDEX IF NOT EXISTS idx_session_shares_session_created
    ON session_shares (session_key, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_session_shares_active
    ON session_shares (id, revoked_at);

CREATE UNIQUE INDEX IF NOT EXISTS idx_session_shares_one_active_per_session
    ON session_shares (session_key)
    WHERE revoked_at IS NULL;
