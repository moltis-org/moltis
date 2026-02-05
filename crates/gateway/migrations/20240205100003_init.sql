-- Gateway tables schema
-- Owned by: moltis-gateway crate
-- Contains: auth, message_log, channels

-- ============================================================================
-- Auth tables
-- ============================================================================

CREATE TABLE IF NOT EXISTS auth_password (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    password_hash TEXT    NOT NULL,
    created_at    TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at    TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS passkeys (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    credential_id BLOB    NOT NULL UNIQUE,
    name          TEXT    NOT NULL,
    passkey_data  BLOB    NOT NULL,
    created_at    TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS api_keys (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    label      TEXT    NOT NULL,
    key_hash   TEXT    NOT NULL,
    key_prefix TEXT    NOT NULL,
    created_at TEXT    NOT NULL DEFAULT (datetime('now')),
    revoked_at TEXT
);

CREATE TABLE IF NOT EXISTS auth_sessions (
    token      TEXT PRIMARY KEY,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS env_variables (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    key        TEXT    NOT NULL UNIQUE,
    value      TEXT    NOT NULL,
    created_at TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT    NOT NULL DEFAULT (datetime('now'))
);

-- ============================================================================
-- Message Log
-- ============================================================================

CREATE TABLE IF NOT EXISTS message_log (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id     TEXT    NOT NULL,
    channel_type   TEXT    NOT NULL,
    peer_id        TEXT    NOT NULL,
    username       TEXT,
    sender_name    TEXT,
    chat_id        TEXT    NOT NULL,
    chat_type      TEXT    NOT NULL,
    body           TEXT    NOT NULL,
    access_granted INTEGER NOT NULL DEFAULT 0,
    created_at     INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_message_log_account_created
    ON message_log (account_id, created_at DESC);

-- ============================================================================
-- Channels
-- ============================================================================

CREATE TABLE IF NOT EXISTS channels (
    account_id   TEXT    PRIMARY KEY,
    channel_type TEXT    NOT NULL DEFAULT 'telegram',
    config       TEXT    NOT NULL,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
