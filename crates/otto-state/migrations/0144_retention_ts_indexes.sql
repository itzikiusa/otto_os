-- Index-driven retention (r3-06-05, r3-07-07/08).
--
-- The hourly agent_trail prune recomputed ROW_NUMBER() over the WHOLE table
-- for every 500-row chunk, under the write lock (2 ms → 110 ms per chunk at
-- 412k rows), and the work_events prune found its items with a full-table
-- GROUP BY. Both now start from the rows that can actually matter:
--   * agent_trail: only sessions that received rows since the last pass can
--     have grown past their cap → `WHERE ts >= ?` on this index.
--   * work_events: only items that own rows older than the window can lose
--     any → `WHERE ts < ?` on this index.
-- Per-session / per-item deletes then walk the existing
-- (session_id, ts, id) / (work_item_id, ts) indexes.
CREATE INDEX IF NOT EXISTS idx_agent_trail_ts ON agent_trail(ts);
CREATE INDEX IF NOT EXISTS idx_work_events_ts ON work_events(ts);
