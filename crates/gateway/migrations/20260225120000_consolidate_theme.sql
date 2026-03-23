ALTER TABLE agents ADD COLUMN theme TEXT;
UPDATE agents SET theme = TRIM(COALESCE(vibe, '') || ' ' || COALESCE(creature, ''))
  WHERE creature IS NOT NULL OR vibe IS NOT NULL;
UPDATE agents SET theme = NULL WHERE theme = '' OR theme = ' ';
ALTER TABLE agents DROP COLUMN creature;
ALTER TABLE agents DROP COLUMN vibe;
