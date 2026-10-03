-- Personal agents: permission modes, standing goals, custom rules, primary agent.
--
-- * Each schedule carries its OWN permission set: `read_only` (the run's
--   session is confined — no mutating otto.* tools, no sends, no writes outside
--   the agent folder) or `directed` (normal approval + auto-approve rules).
-- * Each run records the mode it ran under (`proactive` | `directed` |
--   `scheduled`), whether its session was confined read-only, and, for a
--   proactive run, the standing goal it worked on.
-- * `personal_agent_autonomy` holds the per-agent autonomy config as one JSON
--   document (proactive budget, standing goals, custom rules, primary flag) —
--   separate from `personal_agents` like `personal_agent_context`, so the hot
--   CRUD row stays unchanged. Missing row ⇒ every default (proactive off).
ALTER TABLE personal_agent_schedules ADD COLUMN permission TEXT NOT NULL DEFAULT 'directed';
ALTER TABLE personal_agent_runs ADD COLUMN mode TEXT NOT NULL DEFAULT 'directed';
ALTER TABLE personal_agent_runs ADD COLUMN goal_id TEXT;
ALTER TABLE personal_agent_runs ADD COLUMN read_only INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS personal_agent_autonomy (
    agent_id    TEXT PRIMARY KEY REFERENCES personal_agents(id) ON DELETE CASCADE,
    config_json TEXT NOT NULL DEFAULT '{}',
    updated_at  TEXT NOT NULL
);
