-- Add optional known_hosts pinning per managed SSH target.
ALTER TABLE ssh_targets ADD COLUMN known_host TEXT;
