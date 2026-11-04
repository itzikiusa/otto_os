// What a runtime thread's view derives from its projection (projection.ts).
// Ported from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
// - packages/client-runtime/src/state/threadRequests.ts
//   (`derivePendingThreadRequests`) — a pending request pairs with the last
//   `approval_request` item naming it, answerable only while `live`;
// - apps/web/src/session-logic.ts (`derivePhase`, `projectedWorkEntry`) —
//   the running phase and the one-line work-log labels.

import type { ApprovalOption, ProviderTurn, RequestKind, TurnItem } from '../../../lib/api/types';
import type { RuntimeProjection } from './projection';

export interface PendingApproval {
  requestId: string;
  kind: RequestKind;
  prompt: string | null;
  reason: string | null;
  options: ApprovalOption[];
  /** False once the provider process is gone (`not_resumable`). */
  canRespond: boolean;
}

export function pendingApprovals(p: RuntimeProjection): PendingApproval[] {
  const out: PendingApproval[] = [];
  for (const r of p.requests) {
    if (r.status !== 'pending') continue;
    const item = p.items.findLast((i) => i.type === 'approval_request' && i.request_id === r.id);
    const approval = item?.type === 'approval_request' ? item : null;
    out.push({
      requestId: r.id,
      kind: r.kind,
      prompt: approval?.prompt ?? null,
      reason: approval?.reason ?? null,
      options: approval?.options ?? [],
      canRespond: r.response_capability.type === 'live',
    });
  }
  return out;
}

/** The newest turn (by ordinal). */
export function latestTurn(p: RuntimeProjection): ProviderTurn | null {
  let best: ProviderTurn | null = null;
  for (const t of p.turns) if (!best || t.ordinal > best.ordinal) best = t;
  return best;
}

/** A turn is in flight: Stop shows and the composer queues nothing. */
export function isRunning(p: RuntimeProjection): boolean {
  const t = latestTurn(p);
  return t !== null && (t.status === 'pending' || t.status === 'running');
}

/** One timeline row: a chat message, or a run of consecutive work items
 *  (T3's work-log group between messages). */
export type TimelineRow =
  | { kind: 'message'; item: Extract<TurnItem, { type: 'user_message' | 'assistant_message' }> }
  | { kind: 'work'; id: string; items: TurnItem[] };

export function timelineRows(items: readonly TurnItem[]): TimelineRow[] {
  const rows: TimelineRow[] = [];
  for (const item of items) {
    if (item.type === 'user_message' || item.type === 'assistant_message') {
      rows.push({ kind: 'message', item });
      continue;
    }
    const last = rows[rows.length - 1];
    if (last?.kind === 'work') last.items.push(item);
    else rows.push({ kind: 'work', id: item.id, items: [item] });
  }
  return rows;
}

/** T3's one-line work-log label (`projectedWorkEntry`). */
export function workLabel(item: TurnItem): string {
  const title = item.title?.trim() || null;
  switch (item.type) {
    case 'reasoning':
      return title ?? 'Thinking';
    case 'command_execution':
      return item.input || title || 'Ran command';
    case 'file_change':
      return title ?? `Changed ${item.file_name}`;
    case 'web_search':
      return title ?? (item.patterns?.length ? `Searched ${item.patterns.join(', ')}` : 'Searched the web');
    case 'dynamic_tool':
      return title ?? item.tool_name;
    case 'todo_list': {
      const done = item.steps.filter((s) => s.status === 'completed').length;
      const current = item.steps.find((s) => s.status === 'running') ?? item.steps.find((s) => s.status === 'pending');
      return `Tasks ${done}/${item.steps.length}${current ? ` · ${current.text}` : ''}`;
    }
    case 'approval_request':
      return title ?? item.prompt ?? 'Approval requested';
    case 'system_notice':
      return item.message;
    case 'error':
      return item.failure.class === 'usage_limit' ? 'Usage limit reached' : title || 'Provider error';
    default:
      return title ?? item.type.replaceAll('_', ' ');
  }
}
