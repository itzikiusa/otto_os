-- Smart mode: when a production was started (its place in the line of
-- productions waiting for the engine), kept so a daemon restart starts them
-- again in that order. A nullable column only: the previous build never
-- reads it (its INSERT OR REPLACE writes NULL).
ALTER TABLE media_productions ADD COLUMN queued_at TEXT;
