-- Add thread_id to channel_sessions for Telegram forum-topic session isolation.
-- SQLite cannot alter primary keys, so recreate the table.

CREATE TABLE IF NOT EXISTS channel_sessions_new (
    channel_type TEXT    NOT NULL,
    account_id   TEXT    NOT NULL,
    chat_id      TEXT    NOT NULL,
    thread_id    TEXT    NOT NULL DEFAULT '',
    session_key  TEXT    NOT NULL,
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY (channel_type, account_id, chat_id, thread_id)
);

INSERT OR IGNORE INTO channel_sessions_new (channel_type, account_id, chat_id, thread_id, session_key, updated_at)
    SELECT channel_type, account_id, chat_id, '', session_key, updated_at
    FROM channel_sessions;

DROP TABLE IF EXISTS channel_sessions;

ALTER TABLE channel_sessions_new RENAME TO channel_sessions;
