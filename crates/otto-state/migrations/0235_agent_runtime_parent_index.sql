-- Linked thread groups are listed by their parent (`list_linked`, the group
-- root's sub-threads): an index instead of a scan of every runtime session.
-- Previous-build compatible: an index only.
CREATE INDEX IF NOT EXISTS idx_agent_runtime_sessions_parent ON agent_runtime_sessions(parent_id);
