-- Run-history retention (perf W6): `otto_runs`/`otto_run_events`,
-- `swarm_runs`/`swarm_messages` and finished goal loops' iterations had no
-- delete path. The hourly retention job (otto-state `retention_runs.rs`) now
-- ages them out; this adds what it needs.
--
-- * A swarm's lifetime budget (`max_total_runs` / `max_cost_usd`) counts every
--   run it ever made. Pruned runs are first folded into these per-swarm
--   rollups, and `swarm_spend` adds them back, so retention never refunds a
--   budget.
ALTER TABLE swarms ADD COLUMN pruned_runs INTEGER NOT NULL DEFAULT 0;
ALTER TABLE swarms ADD COLUMN pruned_cost_usd REAL NOT NULL DEFAULT 0;

-- Index-driven retention scans (no full-table scan per pass).
CREATE INDEX IF NOT EXISTS idx_swarm_messages_created ON swarm_messages(created_at);
CREATE INDEX IF NOT EXISTS idx_swarm_runs_finished
    ON swarm_runs(finished_at) WHERE finished_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_goal_loops_finished
    ON goal_loops(finished_at) WHERE finished_at IS NOT NULL;

-- The daily workflow-run prune checks `NOT EXISTS (… scheduled_task_runs
-- WHERE workflow_run_id = ?)` per candidate; that column had no index.
CREATE INDEX IF NOT EXISTS idx_str_wf_run
    ON scheduled_task_runs(workflow_run_id) WHERE workflow_run_id IS NOT NULL;
