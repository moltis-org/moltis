-- Add scopes column to api_keys table.
-- NULL or empty array means full access (operator.admin) for backward compatibility.
ALTER TABLE api_keys ADD COLUMN scopes TEXT;
