-- Goals are workflows: where a workflow came from, and run-level loop
-- limits raised by Resume / Extend. All additive and nullable, all
-- server-set; the previous build never names these columns.
ALTER TABLE workflows ADD COLUMN origin TEXT;          -- NULL (authored) | 'goal'
ALTER TABLE workflows ADD COLUMN origin_ref TEXT;      -- 'review:<id>' | 'insights' | 'goal_loop:<id>' | 'mcp' | NULL
-- {"loops": {"<node id>": {max_iterations?, max_active_s?, unblocked_at?}}}
ALTER TABLE workflow_runs ADD COLUMN limits_json TEXT;
CREATE INDEX IF NOT EXISTS idx_workflows_ws_origin_ref
    ON workflows (workspace_id, origin_ref) WHERE origin_ref IS NOT NULL;
