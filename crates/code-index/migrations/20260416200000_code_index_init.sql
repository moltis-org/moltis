-- Initial schema for code-index SQLite+FTS5 backend.
-- Stores code chunks with optional quantized embeddings and
-- FTS5 full-text index for keyword search.

CREATE TABLE IF NOT EXISTS code_chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL,
    file_path TEXT NOT NULL,
    chunk_index INTEGER NOT NULL,
    content TEXT NOT NULL,
    embedding BLOB,  -- Quantized i8 vector
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(project_id, file_path, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_chunks_project ON code_chunks(project_id);
CREATE INDEX IF NOT EXISTS idx_chunks_file ON code_chunks(project_id, file_path);

-- FTS5 virtual table for keyword search (porter stemmer).
CREATE VIRTUAL TABLE IF NOT EXISTS code_chunks_fts USING fts5(
    content,
    content_rowid=rowid,
    tokenize='porter'
);

-- Triggers to keep FTS5 index in sync with code_chunks.
CREATE TRIGGER IF NOT EXISTS code_chunks_fts_insert
AFTER INSERT ON code_chunks
BEGIN
    INSERT INTO code_chunks_fts(rowid, content) VALUES (NEW.id, NEW.content);
END;

CREATE TRIGGER IF NOT EXISTS code_chunks_fts_delete
AFTER DELETE ON code_chunks
BEGIN
    DELETE FROM code_chunks_fts WHERE rowid = OLD.id;
END;
