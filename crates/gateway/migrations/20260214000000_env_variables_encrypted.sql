-- Add encrypted flag to env_variables for vault integration.
-- 0 = plaintext, 1 = encrypted by vault.
ALTER TABLE env_variables ADD COLUMN encrypted INTEGER NOT NULL DEFAULT 0;
