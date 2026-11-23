// The live list behind `mcpAsks.ts`: pending `mcp_approvals` cards, kept
// fresh by `mcp_approval_changed` while any thread shows (one fetch for every
// open pane), and the answer — approve / deny through the card's decide
// route, "Always" adding a workspace auto-approve rule first.
import { api } from '../../../lib/api/client';
import { mcpCpApi } from '../../../lib/api/mcp';
import type { ApprovalDecision, McpApproval, McpAutoApproveList } from '../../../lib/api/types';
import { liveQuery } from '../../../lib/live';
import type { Poller } from '../../../lib/poll';
import { auth } from '../../../lib/stores/auth.svelte';
import { resourceAccess } from '../../../lib/stores/resource-access.svelte';
import { canDecideTriageApproval } from '../../../shell/triageModel';
import { askAsPending, askDecision, canAlways, isMcpAsk, MCP_ASK, threadAsks } from './mcpAsks';
import type { PendingApproval } from './timeline';

class McpAsks {
  all = $state<McpApproval[]>([]);
  /** Tools a rule may only cover with the second toggle (never "Always"). */
  private irreversible = $state<ReadonlySet<string>>(new Set());
  private users = 0;
  private poller: Poller | null = null;

  /** Keep the list live while the caller (a thread view's `$effect`) shows. */
  hold(): () => void {
    if (this.users++ === 0 && auth.can('mcp', 'view')) {
      this.poller = liveQuery({ run: (signal) => this.load(signal), on: ['mcp_approval_changed'], fallbackMs: 10_000 });
      if (auth.can('mcp', 'admin')) {
        void api.get<McpAutoApproveList>('/mcp/auto-approve').then(
          (l) => (this.irreversible = new Set(l.categories.flatMap((c) => c.tools.filter((t) => t.irreversible).map((t) => t.name.replace(/^otto\./, ''))))),
          () => undefined,
        );
      }
    }
    return () => {
      if (--this.users > 0) return;
      this.poller?.stop();
      this.poller = null;
    };
  }

  private async load(signal: AbortSignal): Promise<void> {
    this.all = (await api.get<McpApproval[]>('/mcp/approvals?status=pending', signal)) ?? [];
    for (const a of this.all) if (a.server_id) void resourceAccess.load('mcp_server', a.server_id, a.tool ?? undefined);
  }

  /** This thread's cards as dock rows. */
  pending(sessionId: string): McpApproval[] {
    return threadAsks(this.all, sessionId);
  }

  asRow(a: McpApproval): PendingApproval {
    const canDecide = canDecideTriageApproval(
      a,
      auth.me?.id,
      (id, tool) => resourceAccess.can('mcp_server', id, 'approve', 'mcp', 'admin', tool),
      () => auth.can('mcp', 'admin'),
    );
    return askAsPending(a, { canDecide, always: canDecide && canAlways(a, this.irreversible, auth.can('mcp', 'admin')) });
  }

  /** Answer a dock row (`mcp:<id>`). Throws on failure (the caller toasts). */
  async answer(rowId: string, decision: ApprovalDecision): Promise<void> {
    const a = this.all.find((x) => MCP_ASK + x.id === rowId);
    if (!a || !isMcpAsk(rowId)) return;
    const { approved, always } = askDecision(decision);
    if (always && a.tool && a.workspace_id) {
      await api.post('/mcp/auto-approve', { scope: 'workspace', workspace_id: a.workspace_id, target_kind: 'tool', target: a.tool });
    }
    await mcpCpApi.cpDecide(a.id, { approved });
    this.all = this.all.filter((x) => x.id !== a.id);
  }
}

export const mcpAsks = new McpAsks();
