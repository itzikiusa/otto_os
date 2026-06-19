-- Agent Swarm budget guardrails (D3). The autonomous coordinator previously had
-- only a concurrency cap (max_parallel_sessions); a re-queueing / handoff-of-
-- handoff loop could spend tokens forever with no off-switch. These columns give
-- each swarm a real, persisted budget. NULL on any column = unlimited for that
-- dimension (power-user opt-out); new swarms get sensible non-null defaults from
-- the service / presets so a runaway swarm self-stops.
--
--   max_total_runs    lifetime run count for the swarm (counts swarm_runs rows)
--   max_runtime_secs  wall-clock budget since the swarm was created
--   max_cost_usd      summed SUM(cost_usd) across runs — SOFT cap (cost may be 0
--                     until usage attribution lands)
--   max_attempts      per-task attempt ceiling: after N runs for a task it is
--                     marked blocked instead of being re-queued forever
ALTER TABLE swarms ADD COLUMN max_total_runs   INTEGER;
ALTER TABLE swarms ADD COLUMN max_runtime_secs INTEGER;
ALTER TABLE swarms ADD COLUMN max_cost_usd     REAL;
ALTER TABLE swarms ADD COLUMN max_attempts     INTEGER;
