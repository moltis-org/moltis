-- Add session_key to track which session a cron run used.
ALTER TABLE cron_runs ADD COLUMN session_key TEXT;

CREATE INDEX IF NOT EXISTS idx_cron_runs_session_key
    ON cron_runs(session_key);
