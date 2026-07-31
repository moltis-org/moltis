-- Host-bound credentials for authenticated HTTPS Git repositories.
CREATE TABLE IF NOT EXISTS git_https_credentials (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    host        TEXT    NOT NULL,
    username    TEXT    NOT NULL,
    token       TEXT    NOT NULL,
    encrypted   INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    UNIQUE(host, username)
);
