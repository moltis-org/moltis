-- Memory system schema

CREATE TABLE IF NOT EXISTS files (
    path       TEXT    NOT NULL PRIMARY KEY,
    source     TEXT    NOT NULL,
    hash       TEXT    NOT NULL,
    mtime      INTEGER NOT NULL,
    size       INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS chunks (
    id         TEXT    NOT NULL PRIMARY KEY,
    path       TEXT    NOT NULL,
    source     TEXT    NOT NULL,
    start_line INTEGER NOT NULL,
    end_line   INTEGER NOT NULL,
    hash       TEXT    NOT NULL,
    model      TEXT    NOT NULL,
    text       TEXT    NOT NULL,
    embedding  BLOB,
    updated_at TEXT    NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (path) REFERENCES files(path) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS embedding_cache (
    provider     TEXT NOT NULL,
    model        TEXT NOT NULL,
    provider_key TEXT NOT NULL,
    hash         TEXT NOT NULL,
    embedding    BLOB NOT NULL,
    dims         INTEGER NOT NULL,
    updated_at   TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (provider, model, provider_key, hash)
);

CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
    text,
    content=chunks,
    content_rowid=rowid
);

-- Triggers to keep FTS in sync
CREATE TRIGGER IF NOT EXISTS chunks_ai AFTER INSERT ON chunks BEGIN
    INSERT INTO chunks_fts(rowid, text) VALUES (new.rowid, new.text);
END;

CREATE TRIGGER IF NOT EXISTS chunks_ad AFTER DELETE ON chunks BEGIN
    INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES ('delete', old.rowid, old.text);
END;

CREATE TRIGGER IF NOT EXISTS chunks_au AFTER UPDATE ON chunks BEGIN
    INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES ('delete', old.rowid, old.text);
    INSERT INTO chunks_fts(rowid, text) VALUES (new.rowid, new.text);
END;
