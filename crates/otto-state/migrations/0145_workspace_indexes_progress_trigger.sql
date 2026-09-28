-- Data-layer hot paths (r3-07-03/04/05).
--
-- 1. One row rewrite per workflow progress write. Repository writes now set
--    nodes_json, progress_json and rev = rev + 1 in ONE statement; the 0130
--    trigger then NULLed progress_json again (a second full-row rewrite, the
--    overflow chain included) only for the republish to write it a third
--    time. Keep the invalidation for RAW writes (restores, manual SQL), which
--    are exactly the ones that leave rev unchanged.
DROP TRIGGER IF EXISTS workflow_progress_body;
CREATE TRIGGER workflow_progress_body AFTER UPDATE OF nodes_json ON workflow_runs
WHEN NEW.rev = OLD.rev BEGIN
    UPDATE workflow_runs SET progress_json=NULL, rev=MAX(rev,OLD.rev+1) WHERE id=NEW.id;
END;

-- 2. Workspace-scoped reads that full-scanned (Mission Control builds, the
--    Activity sidebar chips, `list_active_runs`): lead with workspace_id.
CREATE INDEX IF NOT EXISTS idx_swarm_runs_ws_status ON swarm_runs(workspace_id, status, enqueued_at DESC);
CREATE INDEX IF NOT EXISTS idx_workflow_runs_ws_status ON workflow_runs(workspace_id, status, started_at DESC);
CREATE INDEX IF NOT EXISTS idx_agent_tasks_ws ON agent_tasks(workspace_id, session_id, position);
