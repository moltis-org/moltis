-- Webhook tables schema
-- Owned by: moltis-webhooks crate

CREATE TABLE IF NOT EXISTS webhooks (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    name                 TEXT    NOT NULL,
    description          TEXT,
    enabled              INTEGER NOT NULL DEFAULT 1,
    public_id            TEXT    NOT NULL UNIQUE,
    agent_id             TEXT,
    model                TEXT,
    system_prompt_suffix TEXT,
    tool_policy_json     TEXT,
    auth_mode            TEXT    NOT NULL DEFAULT 'static_header',
    auth_config_json     TEXT,
    source_profile       TEXT    NOT NULL DEFAULT 'generic',
    source_config_json   TEXT,
    event_filter_json    TEXT,
    session_mode         TEXT    NOT NULL DEFAULT 'per_delivery',
    named_session_key    TEXT,
    allowed_cidrs_json   TEXT,
    max_body_bytes       INTEGER NOT NULL DEFAULT 1048576,
    rate_limit_per_minute INTEGER NOT NULL DEFAULT 60,
    delivery_count       INTEGER NOT NULL DEFAULT 0,
    last_delivery_at     TEXT,
    created_at           TEXT    NOT NULL,
    updated_at           TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS webhook_deliveries (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    webhook_id       INTEGER NOT NULL,
    received_at      TEXT    NOT NULL,
    status           TEXT    NOT NULL DEFAULT 'received',
    event_type       TEXT,
    entity_key       TEXT,
    delivery_key     TEXT,
    http_method      TEXT,
    content_type     TEXT,
    remote_ip        TEXT,
    headers_json     TEXT,
    body_size        INTEGER NOT NULL DEFAULT 0,
    body_blob        BLOB,
    session_key      TEXT,
    rejection_reason TEXT,
    run_error        TEXT,
    started_at       TEXT,
    finished_at      TEXT,
    duration_ms      INTEGER,
    input_tokens     INTEGER,
    output_tokens    INTEGER,
    FOREIGN KEY (webhook_id) REFERENCES webhooks(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS webhook_response_actions (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    delivery_id   INTEGER NOT NULL,
    tool_name     TEXT    NOT NULL,
    input_json    TEXT,
    output_json   TEXT,
    status        TEXT    NOT NULL,
    error_message TEXT,
    created_at    TEXT    NOT NULL,
    FOREIGN KEY (delivery_id) REFERENCES webhook_deliveries(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_webhook_id
    ON webhook_deliveries(webhook_id, received_at DESC);

CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_status
    ON webhook_deliveries(status);

CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_delivery_key
    ON webhook_deliveries(delivery_key);

CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_entity_key
    ON webhook_deliveries(entity_key);

CREATE INDEX IF NOT EXISTS idx_webhook_response_actions_delivery
    ON webhook_response_actions(delivery_id);
