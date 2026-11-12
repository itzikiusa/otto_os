-- Whether a workflow trigger works: what its LAST attempt came to, so the
-- Triggers panel can say "Last fired 2 h ago · started" / "Never fired" /
-- "Last attempt rejected: <reason>" instead of nothing. Stamped with one
-- UPDATE where a schedule / chat-binding / webhook trigger fires or is
-- refused. `last_outcome` is 'started' | 'queued' | 'rejected' (no CHECK, so
-- a later outcome kind needs no table rebuild); `last_detail` is the short
-- rejection reason. Nullable and additive: NULL = never fired, which is how
-- the previous build leaves every row.
ALTER TABLE workflow_triggers ADD COLUMN last_fired_at TEXT;
ALTER TABLE workflow_triggers ADD COLUMN last_outcome TEXT;
ALTER TABLE workflow_triggers ADD COLUMN last_detail TEXT;
