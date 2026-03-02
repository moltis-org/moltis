ALTER TABLE sessions ADD COLUMN last_seen_message_count INTEGER NOT NULL DEFAULT 0;
UPDATE sessions SET last_seen_message_count = message_count;
