-- Swarm run lists and the graph's task→session projection both read
-- `WHERE swarm_id = ? ORDER BY enqueued_at DESC`. The only swarm index was
-- (swarm_id, status), so every list sorted the swarm's whole run history in a
-- temp B-tree before applying LIMIT (backlog B6 / SE-09).
CREATE INDEX IF NOT EXISTS idx_swarm_runs_swarm_enqueued ON swarm_runs(swarm_id, enqueued_at DESC);
