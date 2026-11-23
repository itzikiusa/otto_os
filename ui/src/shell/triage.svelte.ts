// Cross-Workspace Triage Bar state (`⌘⇧A`). Aggregates pending MCP approvals
// and waiting sessions/threads across EVERY workspace and provides inline
// Approve / Deny + 1-click cross-workspace jump.

import { api } from '../lib/api/client';
import { mcpCpApi } from '../lib/api/mcp';
import type { McpApproval, Session } from '../lib/api/types';
import { latestOnly } from '../lib/latest';
import { loadErrorText } from '../lib/loadError';
import { router } from '../lib/router.svelte';
import { auth } from '../lib/stores/auth.svelte';
import { resourceAccess } from '../lib/stores/resource-access.svelte';
import { runtimeWaiting } from '../lib/stores/runtimeAttention.svelte';
import { ws } from '../lib/stores/workspace.svelte';
import { toasts } from '../lib/toast.svelte';
import {
  buildTriageItems,
  canDecideTriageApproval,
  triageBlockedReason,
  type TriageItem,
} from './triageModel';

class TriageStore {
  open = $state(false);
  approvals: McpApproval[] = $state([]);
  fetchedSessions: Session[] = $state([]);
  loading = $state(false);
  error = $state('');
  deciding: Record<string, 'approve' | 'deny'> = $state({});
  decideErrors: Record<string, string> = $state({});

  private seq = latestOnly();

  items: TriageItem[] = $derived(
    buildTriageItems({
      approvals: this.approvals,
      sessions: ws.sessions,
      otherWsSessions: this.fetchedSessions.length > 0 ? this.fetchedSessions : ws.otherWsSessions,
      needsYou: ws.needsYou,
      runtimeWaiting: runtimeWaiting(),
      workspaces: ws.workspaces,
      currentWorkspaceId: ws.currentId,
    }),
  );

  show(): void {
    this.open = true;
    void this.refresh();
  }

  close(): void {
    this.open = false;
  }

  toggle(): void {
    if (this.open) this.close();
    else this.show();
  }

  async refresh(): Promise<boolean> {
    const t = this.seq.begin();
    this.loading = true;
    this.error = '';
    try {
      const canMcp = auth.can('mcp', 'view');
      const [appr, sessions] = await Promise.all([
        canMcp
          ? api.get<McpApproval[]>('/mcp/approvals?status=pending', t.signal)
          : Promise.resolve([] as McpApproval[]),
        api.get<Session[]>('/sessions?archived=false&foreground=true', t.signal).catch(() => [] as Session[]),
      ]);
      if (!t.current) return true;
      this.approvals = appr ?? [];
      this.fetchedSessions = sessions ?? [];
      for (const a of this.approvals) {
        if (a.server_id) void resourceAccess.load('mcp_server', a.server_id, a.tool ?? undefined);
      }
      return true;
    } catch (e) {
      if (!t.current) return true;
      this.error = loadErrorText(e);
      return false;
    } finally {
      if (t.current) this.loading = false;
    }
  }

  canDecide(a: McpApproval): boolean {
    return canDecideTriageApproval(
      a,
      auth.me?.id,
      (serverId, tool) => resourceAccess.can('mcp_server', serverId, 'approve', 'mcp', 'admin', tool),
      () => auth.can('mcp', 'admin'),
    );
  }

  blockedReason(a: McpApproval): string {
    return triageBlockedReason(a, auth.me?.id);
  }

  async decide(approval: McpApproval, approved: boolean, note?: string | null): Promise<boolean> {
    if (!this.canDecide(approval) || this.deciding[approval.id]) return false;
    this.deciding = { ...this.deciding, [approval.id]: approved ? 'approve' : 'deny' };
    if (approval.id in this.decideErrors) {
      const next = { ...this.decideErrors };
      delete next[approval.id];
      this.decideErrors = next;
    }
    try {
      const trimmed = note?.trim();
      await mcpCpApi.cpDecide(approval.id, { approved, ...(trimmed ? { note: trimmed } : {}) });
      this.approvals = this.approvals.filter((x) => x.id !== approval.id);
      toasts.success(approved ? 'Approved' : 'Denied', approval.title);
      return true;
    } catch (e) {
      this.decideErrors = { ...this.decideErrors, [approval.id]: loadErrorText(e) };
      return false;
    } finally {
      const next = { ...this.deciding };
      delete next[approval.id];
      this.deciding = next;
    }
  }

  async jumpTo(item: TriageItem): Promise<void> {
    this.close();
    if (item.workspaceId && item.workspaceId !== ws.currentId) {
      try {
        await ws.select(item.workspaceId);
      } catch {
        // Best-effort workspace switch; fall through to navigation.
      }
    }
    if (item.kind === 'session' && item.sessionId) {
      ws.navigateToSession(item.sessionId);
    } else if (item.route) {
      router.go(item.route);
    }
  }
}

export const triage = new TriageStore();
