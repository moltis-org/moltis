ALTER TABLE connector_datasets ADD COLUMN instruction TEXT;
ALTER TABLE connector_datasets ADD COLUMN plan_revision INTEGER NOT NULL DEFAULT 1;
ALTER TABLE connector_datasets ADD COLUMN synced_plan_revision INTEGER;

ALTER TABLE connector_items ADD COLUMN search_text TEXT NOT NULL DEFAULT '';

CREATE TABLE IF NOT EXISTS connector_source_states (
    dataset_id             TEXT    NOT NULL REFERENCES connector_datasets(id) ON DELETE CASCADE,
    remote_id              TEXT    NOT NULL,
    remote_version         TEXT,
    disposition            TEXT    NOT NULL CHECK (disposition IN ('included', 'filtered')),
    filter_reason          TEXT,
    evaluated_plan_revision INTEGER NOT NULL CHECK (evaluated_plan_revision > 0),
    last_seen_run_id       TEXT    NOT NULL,
    observed_at            INTEGER NOT NULL,
    PRIMARY KEY (dataset_id, remote_id)
);

CREATE INDEX IF NOT EXISTS idx_connector_source_states_run
    ON connector_source_states(dataset_id, last_seen_run_id, disposition);

CREATE VIRTUAL TABLE IF NOT EXISTS connector_items_fts USING fts5(
    search_text,
    content='connector_items',
    content_rowid='rowid',
    tokenize='unicode61'
);

CREATE TRIGGER IF NOT EXISTS connector_items_fts_insert AFTER INSERT ON connector_items BEGIN
    INSERT INTO connector_items_fts(rowid, search_text)
    VALUES (new.rowid, new.search_text);
END;

CREATE TRIGGER IF NOT EXISTS connector_items_fts_delete AFTER DELETE ON connector_items BEGIN
    INSERT INTO connector_items_fts(connector_items_fts, rowid, search_text)
    VALUES ('delete', old.rowid, old.search_text);
END;

CREATE TRIGGER IF NOT EXISTS connector_items_fts_update AFTER UPDATE OF search_text ON connector_items BEGIN
    INSERT INTO connector_items_fts(connector_items_fts, rowid, search_text)
    VALUES ('delete', old.rowid, old.search_text);
    INSERT INTO connector_items_fts(rowid, search_text)
    VALUES (new.rowid, new.search_text);
END;

INSERT INTO connector_items_fts(connector_items_fts) VALUES ('rebuild');
