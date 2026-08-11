PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS connector_accounts (
    id         TEXT    PRIMARY KEY,
    kind       TEXT    NOT NULL,
    name       TEXT    NOT NULL,
    config     TEXT    NOT NULL CHECK (json_valid(config)),
    enabled    INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS connector_datasets (
    id               TEXT    PRIMARY KEY,
    account_id       TEXT    NOT NULL REFERENCES connector_accounts(id) ON DELETE CASCADE,
    name             TEXT    NOT NULL,
    config           TEXT    NOT NULL CHECK (json_valid(config)),
    schedule_minutes INTEGER CHECK (schedule_minutes IS NULL OR schedule_minutes > 0),
    projection_jsonl INTEGER NOT NULL DEFAULT 0 CHECK (projection_jsonl IN (0, 1)),
    projection_md    INTEGER NOT NULL DEFAULT 0 CHECK (projection_md IN (0, 1)),
    enabled          INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    last_sync_at     INTEGER,
    next_sync_at     INTEGER,
    last_error       TEXT,
    item_count       INTEGER NOT NULL DEFAULT 0 CHECK (item_count >= 0),
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS connector_items (
    id             TEXT    PRIMARY KEY,
    dataset_id     TEXT    NOT NULL REFERENCES connector_datasets(id) ON DELETE CASCADE,
    remote_id      TEXT    NOT NULL,
    kind           TEXT    NOT NULL,
    remote_version TEXT,
    occurred_at    TEXT,
    updated_at     TEXT,
    body_json      TEXT    NOT NULL CHECK (json_valid(body_json)),
    content_hash   TEXT    NOT NULL,
    created_at     INTEGER NOT NULL,
    stored_at      INTEGER NOT NULL,
    deleted_at     INTEGER,
    UNIQUE(dataset_id, remote_id)
);

CREATE TABLE IF NOT EXISTS connector_sync_runs (
    id         TEXT    PRIMARY KEY,
    dataset_id TEXT    NOT NULL REFERENCES connector_datasets(id) ON DELETE CASCADE,
    status     TEXT    NOT NULL CHECK (status IN ('running', 'succeeded', 'failed')),
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    upserted   INTEGER NOT NULL DEFAULT 0 CHECK (upserted >= 0),
    deleted    INTEGER NOT NULL DEFAULT 0 CHECK (deleted >= 0),
    active     INTEGER NOT NULL DEFAULT 0 CHECK (active >= 0),
    error      TEXT
);

CREATE INDEX IF NOT EXISTS idx_connector_datasets_account
    ON connector_datasets(account_id, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_connector_datasets_due
    ON connector_datasets(enabled, next_sync_at);
CREATE INDEX IF NOT EXISTS idx_connector_items_dataset_active
    ON connector_items(dataset_id, deleted_at, occurred_at);
CREATE INDEX IF NOT EXISTS idx_connector_sync_runs_recent
    ON connector_sync_runs(dataset_id, started_at DESC);
CREATE UNIQUE INDEX IF NOT EXISTS idx_connector_sync_runs_one_running
    ON connector_sync_runs(dataset_id) WHERE status = 'running';
