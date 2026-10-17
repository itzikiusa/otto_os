-- Indexes for queries that full-scanned growing tables on a timer or on a
-- page load (backend performance review, 2026-10). Additive only.

-- The workflow orphan sweep (every 60 s) and chat control list in-flight runs
-- across ALL workspaces: `WHERE status IN ('pending','running') ORDER BY
-- started_at DESC`. Neither existing index leads with status, so each sweep
-- scanned every run row — and `started_at` sits after the 50–200 KB
-- `nodes_json`, so every row's overflow chain was read too. The partial index
-- holds only the handful of live runs.
CREATE INDEX IF NOT EXISTS idx_workflow_runs_active
    ON workflow_runs(started_at DESC)
    WHERE status IN ('pending','running');

-- The assistant tick (every 30 s) expires idle incognito threads:
-- `WHERE incognito = 1 AND COALESCE(last_turn_at, created_at) < ?` scanned
-- every thread of every user. Incognito threads are few and short-lived.
CREATE INDEX IF NOT EXISTS idx_assistant_threads_incognito
    ON assistant_threads(created_at)
    WHERE incognito = 1;

-- Canvas is a global tool: the page lists the caller's scenes across
-- workspaces (`WHERE created_by = ? ORDER BY updated_at DESC`). No index had
-- `created_by`, so every user's scenes were scanned — following each row past
-- its multi-megabyte `doc_json` / `thumbnail` to reach the column.
CREATE INDEX IF NOT EXISTS idx_canvas_scenes_owner_updated
    ON canvas_scenes(created_by, updated_at DESC);

-- A task's latest run (`WHERE task_id = ? ORDER BY enqueued_at DESC LIMIT 1`,
-- correlated per in-progress task in swarm recovery and the run-budget tick)
-- sorted all of the task's runs each time: `idx_swarm_runs_task` has no order.
-- Ascending on purpose: walked backwards it yields `enqueued_at DESC, rowid
-- DESC` — the query's exact tie-break — with no sort at all.
CREATE INDEX IF NOT EXISTS idx_swarm_runs_task_enqueued
    ON swarm_runs(task_id, enqueued_at);
