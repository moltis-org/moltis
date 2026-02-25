-- SQLx wraps migrations in a transaction automatically.
-- PRAGMA foreign_keys cannot be toggled inside a transaction, but the
-- table-swap pattern below does not need it because no FK references channels.

CREATE TABLE channels_new (
    channel_type TEXT    NOT NULL DEFAULT 'telegram',
    account_id   TEXT    NOT NULL,
    config       TEXT    NOT NULL,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY (channel_type, account_id)
);

INSERT INTO channels_new (channel_type, account_id, config, created_at, updated_at)
SELECT
    COALESCE(channel_type, 'telegram'),
    account_id,
    config,
    created_at,
    updated_at
FROM channels;

DROP TABLE channels;
ALTER TABLE channels_new RENAME TO channels;

DROP INDEX IF EXISTS idx_message_log_account_created;
CREATE INDEX IF NOT EXISTS idx_message_log_channel_account_created
    ON message_log (channel_type, account_id, created_at DESC);
