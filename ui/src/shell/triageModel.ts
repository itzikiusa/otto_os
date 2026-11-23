// Pure data model, sorting, permission helpers, and keyboard triage intent
// mapping for the global `⌘⇧A` Cross-Workspace Triage Bar (`TriageBar.svelte`).

import type { McpApproval, Session, WorkspaceWithRole } from '../lib/api/types';

export type TriageItemKind = 'approval' | 'session' | 'thread';

export interface TriageItem {
  id: string;
  kind: TriageItemKind;
  title: string;
  detail: string;
  workspaceId: string | null;
  workspaceName: string;
  at?: string;
  approval?: McpApproval;
  sessionId?: string;
  provider?: string;
  route?: string;
}

export interface RuntimeWaitingLike {
  id: string;
  title?: string | null;
  at?: number | null;
}

export type TriageKeyIntent = 'next' | 'prev' | 'approve' | 'deny' | 'open' | 'refresh' | null;

const AGENT_REQUESTER_KINDS = ['mcp_server', 'gateway', 'agent'];

export function requesterMayDecideApproval(a: Pick<McpApproval, 'requested_by_kind'>): boolean {
  return AGENT_REQUESTER_KINDS.includes(a.requested_by_kind ?? '');
}

export function canDecideTriageApproval(
  a: Pick<McpApproval, 'requested_by' | 'requested_by_kind' | 'server_id' | 'tool'>,
  meId: string | null | undefined,
  canServer: (serverId: string, tool?: string) => boolean,
  canAdmin: () => boolean,
): boolean {
  const allowedRequester = a.requested_by !== meId || requesterMayDecideApproval(a);
  const hasAccess = a.server_id ? canServer(a.server_id, a.tool ?? undefined) : canAdmin();
  return allowedRequester && hasAccess;
}

export function triageBlockedReason(
  a: Pick<McpApproval, 'requested_by' | 'requested_by_kind'>,
  meId: string | null | undefined,
): string {
  return a.requested_by === meId && !requesterMayDecideApproval(a)
    ? 'You raised this request yourself — another admin has to decide it'
    : 'You don’t have approve access to this server';
}

function isThreadSession(s: Session): boolean {
  const meta = s.meta as { scope?: string; ui_scope?: string } | null | undefined;
  return meta?.scope === 'threads' || meta?.ui_scope === 'threads';
}

function ts(iso?: string): number {
  if (!iso) return 0;
  const n = Date.parse(iso);
  return Number.isFinite(n) ? n : 0;
}

/** Combine pending MCP approvals + blocked/waiting sessions & threads across
 *  ALL workspaces. Approvals come first (actionable inline), then waiting
 *  sessions/threads, sorted newest-first within each group. */
export function buildTriageItems(input: {
  approvals: readonly McpApproval[];
  sessions: readonly Session[];
  otherWsSessions: readonly Session[];
  needsYou: Record<string, boolean>;
  runtimeWaiting: readonly RuntimeWaitingLike[];
  workspaces: readonly Pick<WorkspaceWithRole, 'id' | 'name'>[];
  currentWorkspaceId: string | null;
}): TriageItem[] {
  const wsNameById = new Map<string, string>();
  for (const w of input.workspaces) wsNameById.set(w.id, w.name);
  const labelForWs = (id: string | null | undefined): string => {
    if (!id) return 'Global';
    return wsNameById.get(id) ?? 'Workspace';
  };

  const approvalRows: TriageItem[] = [...input.approvals]
    .sort((a, b) => ts(b.created_at) - ts(a.created_at))
    .map((a) => {
      const parts = [a.server_name, a.tool].filter(Boolean);
      return {
        id: `approval:${a.id}`,
        kind: 'approval',
        title: a.title,
        detail: parts.length ? `MCP approval · ${parts.join(' → ')}` : 'MCP approval',
        workspaceId: a.workspace_id ?? null,
        workspaceName: labelForWs(a.workspace_id),
        at: a.created_at,
        approval: a,
        route: 'mcp/activity',
      };
    });

  const seenSessions = new Set<string>();
  const waitingRows: TriageItem[] = [];
  const considerSession = (s: Session) => {
    if (seenSessions.has(s.id) || s.archived) return;
    if (input.needsYou[s.id] !== true) return;
    seenSessions.add(s.id);
    const thread = isThreadSession(s);
    waitingRows.push({
      id: `${thread ? 'thread' : 'session'}:${s.id}`,
      kind: thread ? 'thread' : 'session',
      title: s.title || (thread ? 'Untitled thread' : 'Untitled session'),
      detail: thread ? `Thread · ${s.provider} · waiting for your input` : `Agent · ${s.provider} · waiting for your input`,
      workspaceId: s.workspace_id ?? input.currentWorkspaceId,
      workspaceName: labelForWs(s.workspace_id ?? input.currentWorkspaceId),
      at: s.last_active_at,
      sessionId: s.id,
      provider: s.provider,
      route: thread ? `threads/${encodeURIComponent(s.id)}` : undefined,
    });
  };

  for (const s of input.sessions) considerSession(s);
  for (const s of input.otherWsSessions) considerSession(s);

  for (const r of input.runtimeWaiting) {
    if (seenSessions.has(r.id)) continue;
    seenSessions.add(r.id);
    waitingRows.push({
      id: `thread:${r.id}`,
      kind: 'thread',
      title: r.title?.trim() || 'Waiting thread',
      detail: 'Thread · waiting for your approval or answer',
      workspaceId: input.currentWorkspaceId,
      workspaceName: labelForWs(input.currentWorkspaceId),
      at: r.at ? new Date(r.at).toISOString() : undefined,
      sessionId: r.id,
      route: `threads/${encodeURIComponent(r.id)}`,
    });
  }

  waitingRows.sort((a, b) => ts(b.at) - ts(a.at));
  return [...approvalRows, ...waitingRows];
}

/** Map a keydown inside the Triage Bar to a triage intent (ignoring modified
 *  chords or keys typed into an input/textarea). */
export function triageKeyIntent(e: {
  key: string;
  metaKey?: boolean;
  ctrlKey?: boolean;
  altKey?: boolean;
  isComposing?: boolean;
}): TriageKeyIntent {
  if (e.isComposing || e.metaKey || e.ctrlKey || e.altKey) return null;
  switch (e.key) {
    case 'j':
    case 'J':
    case 'ArrowDown':
      return 'next';
    case 'k':
    case 'K':
    case 'ArrowUp':
      return 'prev';
    case 'a':
    case 'A':
      return 'approve';
    case 'd':
    case 'D':
      return 'deny';
    case 'Enter':
      return 'open';
    case 'r':
    case 'R':
      return 'refresh';
    default:
      return null;
  }
}
