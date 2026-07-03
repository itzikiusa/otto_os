-- Widen workflow_triggers.kind to allow "chat" (channel bindings).
--
-- The `chat` trigger kind was added to route validation (create_trigger in
-- otto-server) and documented, but 0058's `CHECK (kind IN ('schedule',
-- 'webhook', 'event'))` was never widened to match — a `POST
-- /workflows/{id}/triggers` with `kind: "chat"` passed route validation but
-- failed the INSERT's CHECK constraint, so channel bindings could never be
-- persisted.
--
-- SQLite has no `ALTER TABLE ... ALTER CONSTRAINT`, so a CHECK change
-- requires the standard rebuild-and-swap: create a new table with the wider
-- CHECK, copy every row across, drop the old table, rename the new one into
-- place, then recreate both indexes (dropped implicitly with the old table).
-- `workflow_triggers` is only ever the child side of an FK (to `workflows`,
-- ON DELETE CASCADE) and nothing references it back, so this rebuild is
-- safe without touching any other table. sqlx runs migrations inside a
-- transaction, so `PRAGMA foreign_keys` toggles are a no-op here and are
-- intentionally not used.
--
-- kind values:
--   "schedule" — fires on an interval/daily/weekly cadence (spec mirrors the
--                swarm-scheduler format: {cadence, every_min, at, weekday,
--                last_run, enabled})
--   "webhook"  — accepts POST /workflows/{id}/webhook/{token} publicly; the
--                token is stored here and matched in the handler
--   "event"    — subscribes to a named daemon Event kind (e.g. "ReviewChanged")
--                and starts a run when it fires; spec: {event_kind, filter_json}
--   "chat"     — pins a workflow to a channel/chat(/thread); evaluated live by
--                the channels Bridge (not polled) on every inbound message and
--                starts a run from the matching message. spec: {channel, chat,
--                thread?, mention_only?}
CREATE TABLE workflow_triggers_new (
    id          TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('schedule', 'webhook', 'event', 'chat')),
    spec_json   TEXT NOT NULL DEFAULT '{}',
    enabled     INTEGER NOT NULL DEFAULT 1,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO workflow_triggers_new (id, workflow_id, kind, spec_json, enabled, created_at)
    SELECT id, workflow_id, kind, spec_json, enabled, created_at FROM workflow_triggers;

DROP TABLE workflow_triggers;

ALTER TABLE workflow_triggers_new RENAME TO workflow_triggers;

CREATE INDEX IF NOT EXISTS idx_workflow_triggers_workflow
    ON workflow_triggers (workflow_id);

CREATE INDEX IF NOT EXISTS idx_workflow_triggers_kind_enabled
    ON workflow_triggers (kind, enabled);
