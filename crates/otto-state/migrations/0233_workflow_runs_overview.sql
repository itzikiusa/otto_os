-- Workflows on threads: the computed run overview for list rows, the number
-- of steps waiting on a person (the Runs inbox "Needs you" filter), a held
-- run's hold, and the chat status message the run edits.
--
-- Previous-build compatible: nullable columns and one NOT NULL column with a
-- DEFAULT. The old binary never names them; `status` and its CHECK are
-- unchanged. `overview_json` embeds the row's `rev`, so a write that does
-- not refresh it (the old binary, a raw write, an archive restore) makes it
-- stale and readers ignore it.
ALTER TABLE workflow_runs ADD COLUMN overview_json TEXT;
ALTER TABLE workflow_runs ADD COLUMN attention INTEGER NOT NULL DEFAULT 0;
-- {kind: paused|blocked|exhausted, reason, since, by?, node_ids, iteration?};
-- a held run is settled (paused = canceled, blocked/exhausted = error).
ALTER TABLE workflow_runs ADD COLUMN hold_json TEXT;
-- {channel, chat, thread, message_id} of the live status message.
ALTER TABLE workflow_runs ADD COLUMN chat_status_json TEXT;
CREATE INDEX IF NOT EXISTS idx_workflow_runs_ws_attention
    ON workflow_runs (workspace_id, started_at DESC)
    WHERE status = 'running' OR hold_json IS NOT NULL;
