-- Vault metadata: stores wrapped DEK and KDF parameters.
-- Single-row table (id = 1) — one vault per database.

CREATE TABLE IF NOT EXISTS vault_metadata (
    id                   INTEGER PRIMARY KEY CHECK (id = 1),
    version              INTEGER NOT NULL DEFAULT 1,
    kdf_salt             TEXT    NOT NULL,
    kdf_params           TEXT    NOT NULL,
    wrapped_dek          TEXT    NOT NULL,
    recovery_wrapped_dek TEXT,
    recovery_key_hash    TEXT,
    created_at           TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at           TEXT    NOT NULL DEFAULT (datetime('now'))
);
