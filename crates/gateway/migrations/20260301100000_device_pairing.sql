-- Device pairing persistence: paired devices, pair requests, and device tokens.

CREATE TABLE IF NOT EXISTS paired_devices (
    device_id    TEXT PRIMARY KEY,
    display_name TEXT,
    platform     TEXT NOT NULL,
    public_key   TEXT,
    status       TEXT NOT NULL DEFAULT 'active',
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    revoked_at   TEXT
);

CREATE TABLE IF NOT EXISTS pair_requests (
    id           TEXT PRIMARY KEY,
    device_id    TEXT NOT NULL,
    display_name TEXT,
    platform     TEXT NOT NULL,
    public_key   TEXT,
    nonce        TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'pending',
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_pair_requests_status
    ON pair_requests(status) WHERE status = 'pending';

CREATE TABLE IF NOT EXISTS device_tokens (
    token_hash   TEXT PRIMARY KEY,
    token_prefix TEXT NOT NULL,
    device_id    TEXT NOT NULL REFERENCES paired_devices(device_id),
    scopes       TEXT NOT NULL DEFAULT '[]',
    issued_at    TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at   TEXT,
    revoked      INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_device_tokens_device
    ON device_tokens(device_id);
CREATE INDEX IF NOT EXISTS idx_device_tokens_prefix
    ON device_tokens(token_prefix);
