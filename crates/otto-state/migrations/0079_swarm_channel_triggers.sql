-- Bind a workspace channel (Slack/Telegram/webhook) to a swarm so an inbound
-- message launches the team: the message becomes the project goal, the planner
-- seeds tasks, and the coordinator starts. Channels are workspace-level; this
-- maps a (workspace, channel, chat[, keyword]) to a specific swarm.
CREATE TABLE swarm_channel_triggers (
    id            TEXT PRIMARY KEY,
    swarm_id      TEXT NOT NULL,
    workspace_id  TEXT NOT NULL,
    channel       TEXT NOT NULL,                     -- slack | telegram | webhook
    match_chat    TEXT NOT NULL DEFAULT '',          -- '' = any chat in the channel; else a specific chat id
    keyword       TEXT NOT NULL DEFAULT '',          -- '' = any message; else only messages starting with this
    repo_path     TEXT,                              -- repo the launched project's agents work in
    auto_start    INTEGER NOT NULL DEFAULT 1,        -- start the coordinator immediately (else plan only)
    reply         INTEGER NOT NULL DEFAULT 1,        -- post progress/result back to the channel
    enabled       INTEGER NOT NULL DEFAULT 1,
    created_by    TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX idx_swarm_triggers_ws ON swarm_channel_triggers(workspace_id, channel, enabled);
CREATE INDEX idx_swarm_triggers_swarm ON swarm_channel_triggers(swarm_id);
