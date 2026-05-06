-- Add per-key salt for HMAC-SHA256 key hashing.
-- Legacy keys (key_salt IS NULL) fall back to plain SHA-256 verification.
ALTER TABLE api_keys ADD COLUMN key_salt TEXT;
