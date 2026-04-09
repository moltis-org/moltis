-- Add unique constraint on (webhook_id, delivery_key) to prevent TOCTOU
-- dedup races when concurrent identical webhooks arrive simultaneously.
-- delivery_key can be NULL (generic profile without dedup headers), so
-- only non-NULL keys are constrained.
CREATE UNIQUE INDEX IF NOT EXISTS idx_webhook_deliveries_unique_key
    ON webhook_deliveries(webhook_id, delivery_key)
    WHERE delivery_key IS NOT NULL;
