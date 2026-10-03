-- perf F2: one row per completed automation step. The runner used to rewrite
-- the whole run record (snapshot + every step so far) after each step — O(n²)
-- bytes for an n-step run — and every progress poll decoded that whole blob.
-- New runs keep only the header in `record_json` and append here; rows written
-- before this migration keep their steps inside `record_json` (still read).
CREATE TABLE api_automation_run_steps (
    run_id TEXT NOT NULL REFERENCES api_automation_runs(id) ON DELETE CASCADE,
    idx INTEGER NOT NULL,
    dataset_row INTEGER NOT NULL,
    step_result_id TEXT NOT NULL,
    result_json TEXT NOT NULL,
    PRIMARY KEY (run_id, idx)
) WITHOUT ROWID;

-- Summary counters so the run list never decodes step results.
ALTER TABLE api_automation_runs ADD COLUMN steps_total INTEGER NOT NULL DEFAULT 0;
ALTER TABLE api_automation_runs ADD COLUMN steps_passed INTEGER NOT NULL DEFAULT 0;
-- 1 = steps live in api_automation_run_steps; 0 = legacy (inside record_json).
ALTER TABLE api_automation_runs ADD COLUMN steps_in_table INTEGER NOT NULL DEFAULT 0;

-- Per-automation run listing / retention.
CREATE INDEX api_automation_runs_automation ON api_automation_runs(workspace_id, automation_id, id DESC);
