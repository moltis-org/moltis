ALTER TABLE agents ADD COLUMN is_default INTEGER NOT NULL DEFAULT 0;

CREATE UNIQUE INDEX IF NOT EXISTS idx_agents_single_default
ON agents(is_default)
WHERE is_default = 1;
