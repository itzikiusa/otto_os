CREATE TABLE workspace_integrations (
    workspace_id  TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    channel       TEXT NOT NULL CHECK (channel IN ('slack','telegram')),
    enabled       INTEGER NOT NULL DEFAULT 0,
    bot_token_ref TEXT,
    app_token_ref TEXT,
    allowed_users TEXT NOT NULL DEFAULT '',
    agent_reply   INTEGER NOT NULL DEFAULT 0,
    reply_instructions TEXT NOT NULL DEFAULT '',
    channel_id    TEXT NOT NULL DEFAULT '',
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    PRIMARY KEY (workspace_id, channel)
);
