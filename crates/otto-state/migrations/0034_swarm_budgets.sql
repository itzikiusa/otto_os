-- Agent Swarm hardening (deep-dive D3/D8): budget guardrails, a per-task attempt
-- ceiling, and the bookkeeping the Coordinator needs to enforce them. See
-- docs/superpowers/specs/2026-06-18-agent-swarm-design.md.
--
-- All budget limits are nullable = unlimited. The Coordinator checks them on each
-- tick BEFORE scheduling a run; when a limit is exceeded it pauses the swarm with
-- a `pause_reason` instead of spawning more work. `run_started_at` anchors the
-- wall-clock budget (set when a swarm goes active; cleared on pause/abort).

ALTER TABLE swarms ADD COLUMN max_total_runs   INTEGER;          -- cap on total runs ever (NULL = unlimited)
ALTER TABLE swarms ADD COLUMN max_cost_usd      REAL;            -- cap on accumulated spend in USD (NULL = unlimited)
ALTER TABLE swarms ADD COLUMN max_runtime_secs  INTEGER;         -- cap on wall-clock since (last) start (NULL = unlimited)
ALTER TABLE swarms ADD COLUMN max_attempts      INTEGER NOT NULL DEFAULT 3;  -- per-task attempt ceiling
ALTER TABLE swarms ADD COLUMN run_started_at    TEXT;            -- when the swarm last went active (wall-clock anchor)
ALTER TABLE swarms ADD COLUMN pause_reason      TEXT;            -- why the Coordinator auto-paused (budget/limit), else NULL

-- Per-task attempt counter: bumped each time the Coordinator (re-)queues a turn.
-- Once it reaches the swarm's max_attempts and the task still hasn't reached a
-- terminal status, the task is marked `blocked` instead of re-run forever.
ALTER TABLE swarm_tasks ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;
