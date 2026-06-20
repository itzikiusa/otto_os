-- Workflow triggers: schedule, webhook, and event-driven run starters.
-- Each trigger references a workflow and carries a kind-specific spec in JSON.
-- A workflow may have multiple triggers of different kinds.
--
-- kind values:
--   "schedule" — fires on an interval/daily/weekly cadence (spec mirrors the
--                swarm-scheduler format: {cadence, every_min, at, weekday,
--                last_run, enabled})
--   "webhook"  — accepts POST /workflows/{id}/webhook/{token} publicly; the
--                token is stored here and matched in the handler
--   "event"    — subscribes to a named daemon Event kind (e.g. "ReviewChanged")
--                and starts a run when it fires; spec: {event_kind, filter_json}
--
-- human_approval pause/resume is tracked on the run row directly (status =
-- "waiting_approval", approved_by, approval_note on workflow_runs) and does not
-- need a separate trigger row.
CREATE TABLE IF NOT EXISTS workflow_triggers (
    id          TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('schedule', 'webhook', 'event')),
    spec_json   TEXT NOT NULL DEFAULT '{}',
    enabled     INTEGER NOT NULL DEFAULT 1,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_workflow_triggers_workflow
    ON workflow_triggers (workflow_id);

CREATE INDEX IF NOT EXISTS idx_workflow_triggers_kind_enabled
    ON workflow_triggers (kind, enabled);

-- Extend workflow_runs to support the human_approval pause state.
-- Rows are appended on pause; updated in-place on resume.
ALTER TABLE workflow_runs ADD COLUMN waiting_approval INTEGER NOT NULL DEFAULT 0;
ALTER TABLE workflow_runs ADD COLUMN approval_node_id  TEXT;
ALTER TABLE workflow_runs ADD COLUMN approved_by       TEXT;
ALTER TABLE workflow_runs ADD COLUMN approval_note     TEXT;
ALTER TABLE workflow_runs ADD COLUMN approved_at       TEXT;
