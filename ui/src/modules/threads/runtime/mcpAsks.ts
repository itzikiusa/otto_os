// An approval-gated MCP call from a thread's own agent — an `otto.*` tool
// or a governed server's tool waiting on an `mcp_approvals` card — as a
// request of the thread's attention dock: answered there, and the waiting
// call then runs (it holds for `GOVERNED_WAIT_SECS`; only an unanswered card
// falls back to "resubmit after approval"). Pure (unit/threadsMcpAsks.test.ts);
// the live list is `mcpAsks.svelte.ts`.
import type { ApprovalDecision, McpApproval } from '../../../lib/api/types.ts';
import type { PendingApproval } from './timeline.ts';

/** Dock ids of MCP asks carry this prefix (never a `runtime_requests` id). */
export const MCP_ASK = 'mcp:';

export const isMcpAsk = (id: string): boolean => id.startsWith(MCP_ASK);

/** This thread's pending cards, oldest first. */
export function threadAsks(all: readonly McpApproval[], sessionId: string): McpApproval[] {
  return all
    .filter((a) => a.status === 'pending' && a.requested_by_session_id === sessionId)
    .sort((a, b) => a.created_at.localeCompare(b.created_at));
}

const bare = (tool: string): string => tool.replace(/^otto\./, '');

/** "otto · create_pr", "github · create_issue". */
export function askTool(a: Pick<McpApproval, 'server_name' | 'tool' | 'title'>): string {
  return a.tool ? `${a.server_name ?? 'MCP'} · ${bare(a.tool)}` : a.title;
}

/** The arguments in full — the card's redacted copy, pretty-printed: the
 *  person approves what they can read (the block scrolls, height-capped). */
export function argsText(json: string): string | null {
  const raw = json.trim();
  if (!raw || raw === '{}' || raw === 'null') return null;
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}

/** The dock row for a card. "Always" (a workspace auto-approve rule) is
 *  offered only where one can apply: an `otto.*` tool that is not in the
 *  irreversible tier, in a workspace, for an MCP admin. */
export function askAsPending(a: McpApproval, opts: { canDecide: boolean; always: boolean }): PendingApproval {
  const options: PendingApproval['options'] = [
    { decision: 'decline', label: 'Deny' },
    { decision: 'accept', label: 'Allow once' },
  ];
  if (opts.always) options.push({ decision: 'accept_always', label: 'Always allow in this workspace' });
  return {
    requestId: MCP_ASK + a.id,
    kind: 'permission',
    prompt: argsText(a.args_redacted_json),
    reason: a.detail,
    options,
    canRespond: opts.canDecide,
  };
}

/** Whether "Always" can be offered for this card (see {@link askAsPending}). */
export function canAlways(a: McpApproval, irreversible: ReadonlySet<string>, isAdmin: boolean): boolean {
  return isAdmin && !a.server_id && !!a.workspace_id && !!a.tool && !irreversible.has(bare(a.tool));
}

/** What an answer does: approve (after adding the rule, for "Always") or deny. */
export function askDecision(d: ApprovalDecision): { approved: boolean; always: boolean } {
  return { approved: d !== 'decline' && d !== 'cancel', always: d === 'accept_always' };
}
