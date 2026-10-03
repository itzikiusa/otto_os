-- Perf (section 15): covering / range indexes for the swarm coordinator, the
-- Mission Control projector + graph, and the notification center. Additive only.

-- `SUM(cost_usd) … WHERE project_id = ?` runs on every swarm event in the
-- workgraph projector, and the run list filters by project — was a full
-- `SCAN swarm_runs`. Covering, so the SUM never touches the table.
CREATE INDEX IF NOT EXISTS idx_swarm_runs_project_cost ON swarm_runs(project_id, cost_usd);

-- `swarm_spend` / `total_cost` (`COUNT(*), SUM(cost_usd) WHERE swarm_id = ?`)
-- run on every coordinator tick per active swarm. Covering.
CREATE INDEX IF NOT EXISTS idx_swarm_runs_swarm_cost ON swarm_runs(swarm_id, cost_usd);

-- `ready_tasks`: `WHERE swarm_id = ? AND status = 'todo' ORDER BY order_idx`
-- (every tick) and the `status = 'done'` dependency-id read.
CREATE INDEX IF NOT EXISTS idx_swarm_tasks_swarm_status_order
    ON swarm_tasks(swarm_id, status, order_idx);

-- Graph edges are read per workspace; only from/to were indexed.
CREATE INDEX IF NOT EXISTS idx_work_edges_ws ON work_edges(workspace_id);

-- Notification list = (global branch) UNION ALL (mine branch), each a
-- LIMITed range over this index instead of MULTI-INDEX OR + temp B-tree sort.
CREATE INDEX IF NOT EXISTS idx_notifications_user_created
    ON notifications(user_id, created_at DESC, id DESC);

-- Unread badge + "mark all read": only unread rows are indexed.
CREATE INDEX IF NOT EXISTS idx_notifications_unread ON notifications(user_id) WHERE read = 0;
