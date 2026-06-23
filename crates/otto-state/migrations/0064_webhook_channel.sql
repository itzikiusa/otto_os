-- Add the 'webhook' channel kind to workspace_integrations.
--
-- The channel column carries a CHECK constraint and SQLite cannot ALTER a CHECK
-- in place, so we rebuild the table (create-new → copy → drop → rename). The
-- table is a leaf — nothing FK-references it — so the rebuild is FK-safe even
-- with foreign_keys on, and we don't need to toggle the pragma (which is a no-op
-- inside the migration transaction anyway).
--
-- Webhook reuses the existing columns rather than adding webhook-specific ones:
--   bot_token_ref → keychain ref for the webhook secret key
--   channel_id    → optional default reply callback URL
--   allowed_users → optional allowed caller ids
-- so list/upsert/delete keep working unchanged. Only the CHECK widens.

CREATE TABLE workspace_integrations_new (
    workspace_id  TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    channel       TEXT NOT NULL CHECK (channel IN ('slack','telegram','webhook')),
    enabled       INTEGER NOT NULL DEFAULT 0,
    bot_token_ref TEXT,
    app_token_ref TEXT,
    allowed_users TEXT NOT NULL DEFAULT '',
    agent_reply   INTEGER NOT NULL DEFAULT 0,
    reply_instructions TEXT NOT NULL DEFAULT '',
    channel_id    TEXT NOT NULL DEFAULT '',
    preferred_cli TEXT NOT NULL DEFAULT '',
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    PRIMARY KEY (workspace_id, channel)
);

INSERT INTO workspace_integrations_new
    (workspace_id, channel, enabled, bot_token_ref, app_token_ref,
     allowed_users, agent_reply, reply_instructions, channel_id,
     preferred_cli, created_at, updated_at)
SELECT
    workspace_id, channel, enabled, bot_token_ref, app_token_ref,
    allowed_users, agent_reply, reply_instructions, channel_id,
    preferred_cli, created_at, updated_at
FROM workspace_integrations;

DROP TABLE workspace_integrations;

ALTER TABLE workspace_integrations_new RENAME TO workspace_integrations;
