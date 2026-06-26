// MCP server APIs. Two surfaces share this module:
//
//  - `mcpApi` (legacy): per-workspace MCP-server *config* CRUD. Enabled servers
//    are merged into a workspace's `.mcp.json` when an agent session spawns.
//    Nothing is auto-enabled — `enabled` defaults off and is flipped per server.
//  - `mcpCpApi` (control plane): the governed registry — health, discovery,
//    per-tool permissions, allowlists, policy-as-code, approvals, audit, stats,
//    the governed tool tester, and the outward Otto-as-MCP-server admin. New
//    names (cp-prefixed) so the legacy config CRUD above is never disturbed.

import { api } from './client';
import type {
  CreateMcpControlServerReq,
  CreateMcpPolicyReq,
  CreateMcpServerReq,
  DecideMcpApprovalReq,
  EvaluateMcpReq,
  Id,
  ImportMcpPoliciesReq,
  ImportMcpPoliciesResp,
  McpApproval,
  McpAuditQuery,
  McpAllowlistEntry,
  McpCallLogRow,
  McpEvaluatePreview,
  McpInvokeReq,
  McpInvokeResp,
  McpOttoServerStatus,
  McpPolicy,
  McpPolicyExport,
  McpServer,
  McpServerDetail,
  McpServerWithTools,
  McpToolStats,
  McpToolView,
  PatchMcpToolReq,
  SetMcpAllowlistReq,
  UpdateMcpControlServerReq,
  UpdateMcpOttoServerReq,
  UpdateMcpPolicyReq,
  UpdateMcpServerReq,
} from './types';

export const mcpApi = {
  list: (wsId: Id) => api.get<McpServer[]>(`/workspaces/${wsId}/mcp-servers`),
  create: (wsId: Id, body: CreateMcpServerReq) =>
    api.post<McpServer>(`/workspaces/${wsId}/mcp-servers`, body),
  update: (id: Id, body: UpdateMcpServerReq) =>
    api.patch<McpServer>(`/mcp-servers/${id}`, body),
  remove: (id: Id) => api.del<void>(`/mcp-servers/${id}`),
};

/** Build a `?key=value` query string from defined fields only. */
function qs(params: Record<string, string | number | undefined | null>): string {
  const parts: string[] = [];
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== null && v !== '') {
      parts.push(`${encodeURIComponent(k)}=${encodeURIComponent(String(v))}`);
    }
  }
  return parts.length ? `?${parts.join('&')}` : '';
}

/** The governed MCP Control Plane surface (registry → tools → governance). */
export const mcpCpApi = {
  // --- registry / health / discovery ---
  cpList: (wsId: Id) => api.get<McpServerDetail[]>(`/workspaces/${wsId}/mcp/servers`),
  cpCreate: (wsId: Id, body: CreateMcpControlServerReq) =>
    api.post<McpServerDetail>(`/workspaces/${wsId}/mcp/servers`, body),
  cpGet: (id: Id) => api.get<McpServerWithTools>(`/mcp/servers/${id}`),
  cpUpdate: (id: Id, body: UpdateMcpControlServerReq) =>
    api.patch<McpServerDetail>(`/mcp/servers/${id}`, body),
  cpDelete: (id: Id) => api.del<void>(`/mcp/servers/${id}`),
  cpHealth: (id: Id) => api.post<McpServerDetail>(`/mcp/servers/${id}/health`),
  cpDiscover: (id: Id) => api.post<McpToolView[]>(`/mcp/servers/${id}/discover`),

  // --- tools + per-tool permission + the governed invoke (tester / gateway) ---
  cpTools: (id: Id) => api.get<McpToolView[]>(`/mcp/servers/${id}/tools`),
  cpPatchTool: (toolId: Id, body: PatchMcpToolReq) =>
    api.patch<McpToolView>(`/mcp/tools/${toolId}`, body),
  cpInvoke: (serverId: Id, name: string, body: McpInvokeReq) =>
    api.post<McpInvokeResp>(
      `/mcp/servers/${serverId}/tools/${encodeURIComponent(name)}/invoke`,
      body,
    ),

  // --- allowlists ---
  cpAllowlist: (wsId: Id) =>
    api.get<McpAllowlistEntry[]>(`/workspaces/${wsId}/mcp/allowlist`),
  cpSetAllowlist: (wsId: Id, body: SetMcpAllowlistReq) =>
    api.put<void>(`/workspaces/${wsId}/mcp/allowlist`, body),

  // --- policy-as-code ---
  cpPolicies: (wsId?: Id) =>
    api.get<McpPolicy[]>(`/mcp/policies${qs({ workspace_id: wsId })}`),
  cpCreatePolicy: (body: CreateMcpPolicyReq) => api.post<McpPolicy>(`/mcp/policies`, body),
  cpUpdatePolicy: (id: Id, body: UpdateMcpPolicyReq) =>
    api.patch<McpPolicy>(`/mcp/policies/${id}`, body),
  cpDeletePolicy: (id: Id) => api.del<void>(`/mcp/policies/${id}`),
  cpExportPolicies: () => api.get<McpPolicyExport>(`/mcp/policies/export`),
  cpImportPolicies: (body: ImportMcpPoliciesReq) =>
    api.post<ImportMcpPoliciesResp>(`/mcp/policies/import`, body),
  cpEvaluate: (body: EvaluateMcpReq) =>
    api.post<McpEvaluatePreview>(`/mcp/policies/evaluate`, body),

  // --- approvals ---
  cpApprovals: (status?: string) =>
    api.get<McpApproval[]>(`/mcp/approvals${qs({ status })}`),
  cpDecide: (id: Id, body: DecideMcpApprovalReq) =>
    api.post<McpApproval>(`/mcp/approvals/${id}/decide`, body),

  // --- audit + stats ---
  cpAudit: (q: McpAuditQuery = {}) =>
    api.get<McpCallLogRow[]>(
      `/mcp/audit${qs({
        server_id: q.server_id,
        tool: q.tool,
        decision: q.decision,
        limit: q.limit,
        offset: q.offset,
      })}`,
    ),
  cpStats: () => api.get<McpToolStats[]>(`/mcp/stats`),

  // --- outward Otto-as-MCP-server admin ---
  cpOttoServer: () => api.get<McpOttoServerStatus>(`/mcp/otto-server`),
  cpUpdateOttoServer: (body: UpdateMcpOttoServerReq) =>
    api.patch<McpOttoServerStatus>(`/mcp/otto-server`, body),
};
