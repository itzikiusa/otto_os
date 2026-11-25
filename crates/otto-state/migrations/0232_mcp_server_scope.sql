-- Where an MCP server is available. One row per server stays the single
-- place to edit it (one secret blob, one OAuth token set, one health state);
-- its home is still `mcp_servers.workspace_id`. On top of the home:
--   * `all_workspaces = 1`: every workspace, evaluated at read time, so a
--     workspace created later has the server with no further step;
--   * `mcp_server_workspaces`: the other workspaces it was shared with.
-- The one reader is `otto_state::mcp_scope::available_in`.
--
-- Previous-build compatible: an additive column with a DEFAULT and a new
-- table. The old binary keeps reading `workspace_id` only, so every server
-- simply stays in its home workspace there.
ALTER TABLE mcp_servers ADD COLUMN all_workspaces INTEGER NOT NULL DEFAULT 0;

CREATE TABLE mcp_server_workspaces (
    server_id    TEXT NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    PRIMARY KEY (server_id, workspace_id)
);
CREATE INDEX idx_mcp_server_workspaces_ws ON mcp_server_workspaces(workspace_id);

-- The home workspace's row cascades to its servers. A server other workspaces
-- use must not vanish with it: the delete is refused until the server is made
-- home-only (or deleted). Workspaces are archived, not deleted, through the
-- API, so this guards direct deletes.
CREATE TRIGGER mcp_shared_server_home_delete
BEFORE DELETE ON workspaces
WHEN EXISTS (
    SELECT 1 FROM mcp_servers s
    WHERE s.workspace_id = OLD.id
      AND (s.all_workspaces = 1
           OR EXISTS (SELECT 1 FROM mcp_server_workspaces l WHERE l.server_id = s.id))
)
BEGIN
    SELECT RAISE(ABORT, 'this workspace is the home of an MCP server shared with other workspaces: make it available here only, or delete it, first');
END;
