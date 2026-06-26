-- Project-level skill set (layered on team + per-agent skills), the dedicated
-- swarm integration branch worktrees are based on + merged into, and the channel
-- origin a webhook/Slack-launched project replies back to.
ALTER TABLE swarm_projects ADD COLUMN skills_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE swarm_projects ADD COLUMN integration_branch TEXT;
ALTER TABLE swarm_projects ADD COLUMN origin_channel TEXT;   -- slack|telegram|webhook (NULL = not channel-launched)
ALTER TABLE swarm_projects ADD COLUMN origin_chat TEXT;      -- chat/channel id (or webhook callback URL)
ALTER TABLE swarm_projects ADD COLUMN origin_thread TEXT;    -- thread within the chat (optional)
