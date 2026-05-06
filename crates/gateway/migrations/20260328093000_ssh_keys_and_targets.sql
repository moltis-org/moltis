-- Managed outbound SSH identities and named targets for remote exec.
CREATE TABLE IF NOT EXISTS ssh_keys (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL UNIQUE,
    private_key TEXT    NOT NULL,
    public_key  TEXT    NOT NULL,
    fingerprint TEXT    NOT NULL,
    encrypted   INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS ssh_targets (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    label       TEXT    NOT NULL UNIQUE,
    target      TEXT    NOT NULL,
    port        INTEGER,
    auth_mode   TEXT    NOT NULL DEFAULT 'system',
    key_id      INTEGER,
    is_default  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY(key_id) REFERENCES ssh_keys(id)
);
