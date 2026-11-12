-- MCP servers: one list, two run modes (docs/features/mcp-control-plane.md,
-- "Native and governed servers"). `managed` is the mode: 1 = GOVERNED (Otto's
-- gateway, policy, approval, audit), 0 = NATIVE (the agent CLI launches it
-- from `.mcp.json`, nothing governed).
--
-- Every row was `managed = 1` (0077's default), so a stdio server without an
-- Enforced access policy ran BOTH ways: written into `.mcp.json` and also
-- surfaced through the gateway. Those are the servers the workspace MCP-servers
-- page managed before access policies existed; they keep running natively and
-- leave the gateway (a server is never both). Enforced servers were never
-- written to `.mcp.json` and stay governed. No column, no row is added or
-- removed, so the previous build reads the same rows: it already skips an
-- unmanaged Legacy server in the gateway and still writes it to `.mcp.json`.
UPDATE mcp_servers
   SET managed = 0
 WHERE transport = 'stdio'
   AND NOT EXISTS (
       SELECT 1 FROM resource_access_policies p
        WHERE p.resource_kind = 'mcp_server'
          AND p.resource_id = mcp_servers.id
          AND p.mode = 'enforced');
