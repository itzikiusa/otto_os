-- Monotonic per-run revision: bumped on every persisted progress write
-- (update_run, the human-approval pause, and the approve/reject decision).
-- Clients use it to discard stale/out-of-order run snapshots and to apply
-- `workflow_run_updated` WS events in order. Existing rows start at 0.
ALTER TABLE workflow_runs ADD COLUMN rev INTEGER NOT NULL DEFAULT 0;
