-- One-time repair (review 14-daemon-perf P1): every daemon boot used to run
-- `update_status(Reconnectable)` on EVERY dormant session, which also stamped
-- `last_active_at = now`. Live DBs therefore show nearly all `reconnectable`
-- rows carrying the last boot's timestamp, which broke auto-archive
-- (`last_active_at >= cutoff` never ages out) and recency ordering. The boot
-- pass is now set-based and leaves `last_active_at` alone; this restores the
-- old values where the agent trail still knows them.
--
-- The trail keeps the newest 1,000 rows per session, so its max(ts) is the
-- session's real last activity. Only rows whose stamp is NEWER than that are
-- moved back; sessions without trail rows (shells, never-used agents) keep
-- their value. Both columns are written by the same RFC 3339 formatter, so the
-- string comparison orders correctly. `idx_agent_trail_session` serves the
-- per-row max.
UPDATE sessions
SET last_active_at = (
    SELECT max(t.ts) FROM agent_trail t WHERE t.session_id = sessions.id
)
WHERE status = 'reconnectable'
  AND (SELECT max(t.ts) FROM agent_trail t WHERE t.session_id = sessions.id)
      < sessions.last_active_at;
