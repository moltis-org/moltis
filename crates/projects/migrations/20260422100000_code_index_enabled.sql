-- Add code_index_enabled column to projects table
-- Defaults to 1 (enabled) for all existing projects

ALTER TABLE projects ADD COLUMN code_index_enabled INTEGER NOT NULL DEFAULT 1;
