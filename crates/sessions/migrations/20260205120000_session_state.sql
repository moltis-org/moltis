-- Per-session key-value state store for skills and extensions.
CREATE TABLE IF NOT EXISTS session_state (
    session_key TEXT    NOT NULL,
    namespace   TEXT    NOT NULL,
    key         TEXT    NOT NULL,
    value       TEXT    NOT NULL,
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (session_key, namespace, key)
);

CREATE INDEX IF NOT EXISTS idx_session_state_session ON session_state(session_key);
