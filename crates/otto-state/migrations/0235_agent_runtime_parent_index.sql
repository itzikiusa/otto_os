-- A linked group read by its root (`AgentRuntimeRepo::list_linked`): the
-- group cap, a family Stop and a council no longer ride the person's
-- 500-row list. Additive: a previous build ignores the index.
CREATE INDEX IF NOT EXISTS idx_agent_runtime_sessions_parent ON agent_runtime_sessions(parent_id);
