-- Cron tables schema
-- Owned by: moltis-cron crate

CREATE TABLE IF NOT EXISTS cron_jobs (
    id   TEXT PRIMARY KEY,
    data TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cron_runs (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id         TEXT    NOT NULL,
    started_at_ms  INTEGER NOT NULL,
    finished_at_ms INTEGER NOT NULL,
    status         TEXT    NOT NULL,
    error          TEXT,
    duration_ms    INTEGER NOT NULL,
    output         TEXT,
    input_tokens   INTEGER,
    output_tokens  INTEGER,
    FOREIGN KEY (job_id) REFERENCES cron_jobs(id)
);

CREATE INDEX IF NOT EXISTS idx_cron_runs_job_id
    ON cron_runs(job_id, started_at_ms DESC);
