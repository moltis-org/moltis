-- Index for Ed25519 public key lookups during challenge-response auth.
-- No schema changes needed: public_key TEXT already exists in both tables.

CREATE INDEX IF NOT EXISTS idx_paired_devices_public_key
    ON paired_devices(public_key)
    WHERE public_key IS NOT NULL AND status = 'active';

CREATE INDEX IF NOT EXISTS idx_pair_requests_public_key
    ON pair_requests(public_key)
    WHERE public_key IS NOT NULL AND status = 'pending';
