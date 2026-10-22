-- History transcript resolution (backend-performance-4-06): when the root scan
-- for a session with no persisted `transcript_path` finds nothing, stamp the
-- time (epoch seconds) so the History page skips re-scanning that row on every
-- page, filter change and search until a TTL passes. Cleared on the next
-- successful capture. Nullable, additive: an older build ignores the column.
ALTER TABLE sessions ADD COLUMN transcript_missing_at INTEGER;
