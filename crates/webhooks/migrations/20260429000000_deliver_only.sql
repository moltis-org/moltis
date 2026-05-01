-- Add deliver_only mode and template fields.
ALTER TABLE webhooks ADD COLUMN deliver_only INTEGER NOT NULL DEFAULT 0;
ALTER TABLE webhooks ADD COLUMN prompt_template TEXT;
ALTER TABLE webhooks ADD COLUMN deliver_to TEXT;
ALTER TABLE webhooks ADD COLUMN deliver_extra_json TEXT;
