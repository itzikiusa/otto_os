-- Personal agents on the structured runtime (Threads).
--
-- `agent_runtime_sessions.meta`: the server-owned JSON a thread runs under —
-- the same keys a terminal session keeps in `sessions.meta` that the spawn's
-- safety pipeline and the tool policy read (`personal_agent`, `read_only`,
-- `source`, `run_id`, `browser`, `agent_mode`, …). Written once at create by
-- the daemon (never by a client). Nullable: a rolled-back build ignores it
-- and its threads run as plain threads.
ALTER TABLE agent_runtime_sessions ADD COLUMN meta TEXT;

-- `personal_agents.created_by_agent`: the main agent that created this one
-- (`otto.personal_agent_create`), shown on the agent so an agent-made agent
-- is always attributed. NULL = a person created it. A rolled-back build
-- ignores it.
ALTER TABLE personal_agents ADD COLUMN created_by_agent TEXT;
