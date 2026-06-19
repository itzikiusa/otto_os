-- User-managed MCP (Model Context Protocol) servers, per workspace. When an
-- agent session spawns in a workspace, every *enabled* server here is merged
-- into that workspace's `.mcp.json` alongside Otto's own managed entries (e.g.
-- the browser server) — see `otto-sessions::mcp`. Nothing here is ever
-- auto-enabled: a server is written to `.mcp.json` only when the user flips
-- `enabled` on and a session then spawns in the workspace.
--
-- `args` and `env` are JSON: `args` an array of strings, `env` an object of
-- string->string. NOTE: env values are stored in plaintext for now (a server's
-- API keys / tokens belong in the user's own MCP config or, later, in Keychain
-- secret refs). This is documented in the UI; treat it like `.mcp.json` itself
-- (already plaintext on disk in the workspace).
CREATE TABLE mcp_servers (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    -- The key used in `.mcp.json`'s `mcpServers` map; unique within a workspace.
    name         TEXT NOT NULL,
    command      TEXT NOT NULL,
    args_json    TEXT NOT NULL DEFAULT '[]',
    env_json     TEXT NOT NULL DEFAULT '{}',
    enabled      INTEGER NOT NULL DEFAULT 0,
    created_by   TEXT NOT NULL REFERENCES users(id),
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    UNIQUE (workspace_id, name)
);
CREATE INDEX idx_mcp_servers_ws ON mcp_servers(workspace_id);
