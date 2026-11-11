-- Which connected Slack team "act as me" uses when a send names none: a
-- person's global default (workspace_id = '') and per-workspace overrides
-- (a workspace's row wins). Per person, like the xoxp- tokens themselves
-- (Keychain `slack-user-{user}-{team}`); only the team id is stored here.
CREATE TABLE IF NOT EXISTS slack_as_me_defaults (
    user_id      TEXT NOT NULL,
    workspace_id TEXT NOT NULL DEFAULT '',
    team_id      TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    PRIMARY KEY (user_id, workspace_id)
);
