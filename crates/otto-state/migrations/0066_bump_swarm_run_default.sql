-- Raise the default swarm run-budget ceiling from the old auto-default (3000) to
-- the new high backstop (1,000,000,000). The 3000 cap was a runaway-loop guard,
-- not a usage limit, but it was low enough to surface as a visible "runs N/3000"
-- ceiling on long-running teams. NULL stays unlimited; any swarm with an
-- explicit, non-default cap is left untouched — we only rewrite rows still parked
-- at the exact old default value so deliberate per-swarm budgets are preserved.
UPDATE swarms SET max_total_runs = 1000000000 WHERE max_total_runs = 3000;

-- Also drop the old 4h (14400s) wall-clock auto-pause default to unlimited (NULL):
-- a swarm now runs until the user stops it. Same guard — only rows still parked at
-- the exact old default are touched, preserving any deliberate per-swarm runtime cap.
UPDATE swarms SET max_runtime_secs = NULL WHERE max_runtime_secs = 14400;
