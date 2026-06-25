-- Per-scene metadata for Canvas Studio: which agent drives "Ask AI" (provider)
-- and an optional folder path used to group scenes in the UI (section).
-- NULL/empty section = root/ungrouped.
ALTER TABLE canvas_scenes ADD COLUMN provider TEXT NOT NULL DEFAULT 'claude';
ALTER TABLE canvas_scenes ADD COLUMN section TEXT;
