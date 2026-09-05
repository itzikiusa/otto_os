-- Tasks board → agent (design §4.5). `source` says who created the row: the
-- provider's own plan sync ('agent', TodoWrite/TaskCreate) or a human via
-- `POST /sessions/{id}/tasks` ('user'). The full-sync path now replaces only
-- 'agent' rows and merges into 'user' rows by ext_id / normalized title, so
-- board-added tasks survive every plan update with a stable id.
-- `nudge_pending` marks a user task the nudge sweep still has to hand to the
-- agent (one prompt via the PTY once the session is idle); `nudged_at` records
-- when it did.
ALTER TABLE agent_tasks ADD COLUMN source TEXT NOT NULL DEFAULT 'agent';
ALTER TABLE agent_tasks ADD COLUMN description TEXT;
ALTER TABLE agent_tasks ADD COLUMN nudge_pending INTEGER NOT NULL DEFAULT 0;
ALTER TABLE agent_tasks ADD COLUMN nudged_at TEXT;
CREATE INDEX idx_agent_tasks_nudge ON agent_tasks(nudge_pending) WHERE nudge_pending = 1;
