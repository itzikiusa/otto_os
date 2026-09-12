import { api } from '../../lib/api/client';
import type {
  Id,
  McpSessionAttach,
  RotateMcpTokenResp,
  UpdateMcpSessionAttachReq,
} from '../../lib/api/types';

export interface McpGatewayToolRow {
  name: string;
  description?: string | null;
  server_id?: string;
  server_name?: string;
  tool?: string;
}

export const mcpCpExtraApi = {
  rotateToken: (id: Id) => api.post<RotateMcpTokenResp>(`/mcp/tokens/${id}/rotate`),
  sessionAttach: (wsId: Id) =>
    api.get<McpSessionAttach>(`/workspaces/${wsId}/mcp/session-attach`),
  setSessionAttach: (wsId: Id, body: UpdateMcpSessionAttachReq) =>
    api.patch<McpSessionAttach>(`/workspaces/${wsId}/mcp/session-attach`, body),
  gatewayTools: (wsId: Id) =>
    api.get<{ tools: McpGatewayToolRow[] }>(
      `/mcp/gateway/tools?workspace_id=${encodeURIComponent(wsId)}`,
    ),
};
