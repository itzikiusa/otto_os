-- User-maintained context is separate from agent-prunable filesystem memory.
CREATE TABLE personal_agent_context (
    agent_id TEXT PRIMARY KEY REFERENCES personal_agents(id) ON DELETE CASCADE,
    content TEXT NOT NULL DEFAULT '',
    version TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
