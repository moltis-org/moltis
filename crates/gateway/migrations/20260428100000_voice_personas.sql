-- Voice personas: named, reusable voice identities for TTS.
CREATE TABLE IF NOT EXISTS voice_personas (
    id              TEXT PRIMARY KEY,
    label           TEXT NOT NULL,
    description     TEXT,
    provider        TEXT,
    fallback_policy TEXT NOT NULL DEFAULT 'preserve-persona',
    prompt_json     TEXT NOT NULL DEFAULT '{}',
    bindings_json   TEXT NOT NULL DEFAULT '[]',
    is_active       INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

-- At most one persona may be active at a time.
CREATE UNIQUE INDEX IF NOT EXISTS uix_voice_personas_active
    ON voice_personas (is_active) WHERE is_active = 1;
