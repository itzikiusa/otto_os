// Workspace MCP-server config API. Users manage the MCP servers Otto merges
// into a workspace's `.mcp.json` when an agent session spawns there. Nothing is
// auto-enabled — `enabled` defaults off and must be flipped on per server.

import { api } from './client';
import type { CreateMcpServerReq, Id, McpServer, UpdateMcpServerReq } from './types';

export const mcpApi = {
  list: (wsId: Id) => api.get<McpServer[]>(`/workspaces/${wsId}/mcp-servers`),
  create: (wsId: Id, body: CreateMcpServerReq) =>
    api.post<McpServer>(`/workspaces/${wsId}/mcp-servers`, body),
  update: (id: Id, body: UpdateMcpServerReq) =>
    api.patch<McpServer>(`/mcp-servers/${id}`, body),
  remove: (id: Id) => api.del<void>(`/mcp-servers/${id}`),
};
