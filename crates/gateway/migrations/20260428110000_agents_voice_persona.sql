-- Link agents to voice personas for automatic voice switching.
ALTER TABLE agents ADD COLUMN voice_persona_id TEXT REFERENCES voice_personas(id) ON DELETE SET NULL;
