// What a runtime thread is waiting on the person for — a pending approval, a
// question form or a private secret request — as ONE ordered list the
// attention dock (RuntimeAttentionDock.svelte) steps through, directly above
// the composer. The live answer controls exist once, in the dock; the
// timeline keeps the historical row. Pure (unit/threadsAttentionDock.test.ts).
import type { ApprovalDecision, ApprovalOption, McpApproval, RequestKind, RuntimeRequest } from '../../../lib/api/types.ts';
import { askTool, MCP_ASK } from './mcpAsks.ts';

export type AttentionKind = 'approval' | 'question' | 'secret';

export interface AttentionItem {
  kind: AttentionKind;
  /** The request id (`runtime_requests` id, or the secret's `request_id`). */
  id: string;
  /** When the agent asked (ISO), or null when unknown (a secret). */
  at: string | null;
  /** Approvals: the request kind (command, file_change …). */
  requestKind?: RequestKind;
  /** An MCP approval card (`mcpAsks.ts`): the tool it would run. */
  tool?: string;
}

/** Everything pending, in the order it should be answered: a secret first
 *  (it blocks a tool call outright), then approvals and questions oldest
 *  first — the agent asked them in that order. */
export function attentionItems(
  requests: readonly Pick<RuntimeRequest, 'id' | 'kind' | 'status' | 'created_at' | 'response_capability'>[],
  secret: { request_id: string } | null,
  mcp: readonly Pick<McpApproval, 'id' | 'created_at' | 'server_name' | 'tool' | 'title'>[] = [],
): AttentionItem[] {
  const out: AttentionItem[] = mcp.map((a) => ({ kind: 'approval', id: MCP_ASK + a.id, at: a.created_at, requestKind: 'permission', tool: askTool(a) }));
  for (const r of requests) {
    if (r.status !== 'pending') continue;
    if (r.kind === 'question') {
      // A question that can no longer be answered (agent gone) is not shown as a form.
      if (r.response_capability.type !== 'live') continue;
      out.push({ kind: 'question', id: r.id, at: r.created_at ?? null });
    } else {
      out.push({ kind: 'approval', id: r.id, at: r.created_at ?? null, requestKind: r.kind });
    }
  }
  out.sort((a, b) => (Date.parse(a.at ?? '') || Infinity) - (Date.parse(b.at ?? '') || Infinity));
  return secret ? [{ kind: 'secret', id: secret.request_id, at: null }, ...out] : out;
}

/** Which item the dock shows: the one the person picked while it is still
 *  pending, else the first. -1 when nothing waits. */
export function currentIndex(items: readonly AttentionItem[], picked: string | null): number {
  if (items.length === 0) return -1;
  const at = picked ? items.findIndex((i) => i.id === picked) : -1;
  return at >= 0 ? at : 0;
}

/** The id one step forward / back (wrapping), for the dock's ‹ › stepper. */
export function stepId(items: readonly AttentionItem[], picked: string | null, delta: 1 | -1): string | null {
  const at = currentIndex(items, picked);
  if (at < 0) return null;
  return items[(at + delta + items.length) % items.length].id;
}

const VERBS: Record<RequestKind, string> = {
  command: 'run',
  file_change: 'change',
  file_read: 'read',
  permission: 'use',
  mcp_elicitation: 'access',
  question: 'ask',
};

/** The dock's one-line headline: "Codex wants to run" (the target shows next
 *  to it in mono), "Claude asks" or "Claude asks for a secret". */
export function attentionHeadline(item: AttentionItem, agent: string): string {
  if (item.kind === 'secret') return `${agent} asks for a secret`;
  if (item.kind === 'question') return `${agent} asks`;
  if (item.tool) return `${agent} wants to use ${item.tool}`;
  return `${agent} wants to ${VERBS[item.requestKind ?? 'permission']}`;
}

/** The follow pill's words when the reader is scrolled away from a request. */
export function attentionPillText(items: readonly AttentionItem[]): string | null {
  if (items.length === 0) return null;
  return items.some((i) => i.kind !== 'approval') ? 'Needs your answer' : 'Approval needed';
}

/** What a key press does in the dock. Keys only answer when they cannot be
 *  typing: the composer is empty, or focus is on the dock's own buttons. A
 *  question form takes ⌘↵ / Ctrl+↵ as "Send answer" from anywhere inside it. */
export type DockKeyAction = 'accept' | 'decline' | 'send' | 'next' | 'prev' | null;

export interface DockKeyEvent {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  isComposing?: boolean;
}

export function dockKeyAction(
  e: DockKeyEvent,
  ctx: { kind: AttentionKind | null; composerEmpty: boolean; inDock: boolean; inTextField: boolean; count: number },
): DockKeyAction {
  if (!ctx.kind || e.isComposing || e.altKey) return null;
  const mod = e.metaKey || e.ctrlKey;
  if (ctx.kind === 'question') return ctx.inDock && mod && e.key === 'Enter' ? 'send' : null;
  if (ctx.kind !== 'approval') return null;
  // Never while the person is typing: from the composer only when it is
  // empty, inside the dock only from its buttons.
  if (ctx.inDock ? ctx.inTextField : !ctx.composerEmpty) return null;
  if (mod && !e.shiftKey && e.key === 'Enter') return 'accept';
  if (mod && !e.shiftKey && e.key === 'Backspace') return 'decline';
  if (ctx.inDock && !mod && ctx.count > 1 && (e.key === 'ArrowRight' || e.key === 'ArrowLeft')) {
    return e.key === 'ArrowRight' ? 'next' : 'prev';
  }
  return null;
}

/** The decision a keyboard answer sends: the agent's own accept / decline
 *  option when it offers one, else the plain decision (the row's defaults). */
export function keyDecision(options: readonly ApprovalOption[], action: 'accept' | 'decline'): ApprovalDecision | null {
  if (options.length === 0) return action;
  return options.some((o) => o.decision === action) ? action : null;
}

/** "⌘↵" on a Mac, "Ctrl+↵" elsewhere — the dock's keyboard hints. */
export function keyHint(action: 'accept' | 'decline' | 'send', mac: boolean): string {
  const mod = mac ? '⌘' : 'Ctrl+';
  return `${mod}${action === 'decline' ? '⌫' : '↵'}`;
}
