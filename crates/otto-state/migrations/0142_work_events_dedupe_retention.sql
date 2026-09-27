-- Retention support (B8 / SG-05, SE-05).
--
-- 1. One-time, idempotent dedupe of `artifact_added` work events. Before the
--    NULL-ref fix (ffc73f3e) the reconcile sweep re-attached the same artifact
--    every 5 minutes, leaving ~250k identical rows on long-lived installs (one
--    item had 70k). Keep the EARLIEST row per (work_item_id, actor,
--    payload_json), ties on ts broken by id. Re-running deletes nothing.
DELETE FROM work_events
WHERE event_type = 'artifact_added'
  AND id NOT IN (
    SELECT id FROM (
      SELECT id,
             ROW_NUMBER() OVER (
               PARTITION BY work_item_id, actor, payload_json
               ORDER BY ts, id
             ) AS rn
      FROM work_events
      WHERE event_type = 'artifact_added'
    )
    WHERE rn = 1
  );

-- 2. Age indexes for the hourly retention prune (otto-state retention.rs):
--    the existing indexes lead with session/tool/workspace, so a
--    `created_at < cutoff` delete would scan the whole table every hour.
CREATE INDEX IF NOT EXISTS idx_mcp_tool_calls_created ON mcp_tool_calls(created_at);
CREATE INDEX IF NOT EXISTS idx_mcp_call_log_created ON mcp_call_log(created_at);
