// What a runtime thread's view derives from its projection (projection.ts).
// Ported from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
// - packages/client-runtime/src/state/threadRequests.ts
//   (`derivePendingThreadRequests`) — a pending request pairs with the last
//   `approval_request` item naming it, answerable only while `live`;
// - apps/web/src/session-logic.ts (`derivePhase`, `projectedWorkEntry`) —
//   the running phase and the one-line work-log labels;
// - apps/web/src/components/chat/MessagesTimeline.logic.ts + packages/
//   client-runtime/src/work-log/presentation.ts (`summarizeToolGroup`) — tool
//   calls grouped under one summary line, approvals folded into their tool.

import type { ApprovalOption, ProviderTurn, RequestKind, RuntimeRequest, TurnItem } from '../../../lib/api/types';
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

type ApprovalItem = Extract<TurnItem, { type: 'approval_request' }>;

/** A work-log item, with the approval that gated it folded in (T3's
 *  TimelineWorkEntryRow: the approval lives on its tool's row). */
export interface WorkEntry {
  item: TurnItem;
  approval?: ApprovalItem;
}

/** One entry, or tool calls that ran at the same time (T3's work-log group). */
export type WorkPart = { kind: 'item'; entry: WorkEntry } | { kind: 'group'; id: string; entries: WorkEntry[] };

/** One timeline row: a chat message, or a run of consecutive work items
 *  (T3's work-log group between messages). */
export type TimelineRow =
  | { kind: 'message'; item: Extract<TurnItem, { type: 'user_message' | 'assistant_message' }> }
  | { kind: 'work'; id: string; parts: WorkPart[] }
  /** A date separator, only in a thread that spans more than one day. */
  | { kind: 'day'; id: string; label: string }
  | { kind: 'session'; id: string };

const TOOL_TYPES: ReadonlySet<TurnItem['type']> = new Set(['command_execution', 'dynamic_tool', 'web_search', 'file_change']);
const isTool = (i: TurnItem): boolean => TOOL_TYPES.has(i.type);

/** Tool calls whose run times overlap (started before every earlier member
 *  ended; a running one hasn't ended) form one group. */
function groupParallel(entries: WorkEntry[]): WorkPart[] {
  const parts: WorkPart[] = [];
  let run: WorkEntry[] = [];
  let end = -Infinity;
  const flush = (): void => {
    if (run.length > 1) parts.push({ kind: 'group', id: `group:${run[0].item.id}`, entries: run });
    else if (run.length === 1) parts.push({ kind: 'item', entry: run[0] });
    run = [];
    end = -Infinity;
  };
  for (const e of entries) {
    const start = isTool(e.item) && e.item.started_at ? Date.parse(e.item.started_at) : NaN;
    if (Number.isNaN(start)) {
      flush();
      parts.push({ kind: 'item', entry: e });
      continue;
    }
    if (start >= end) flush();
    run.push(e);
    // A call still running hasn't ended; one that stopped without an end time ended when last updated.
    const live = e.item.status === 'running' || e.item.status === 'pending' || e.item.status === 'waiting';
    end = Math.max(end, Date.parse(e.item.completed_at ?? (live ? '' : e.item.updated_at)) || Infinity);
  }
  flush();
  return parts;
}

/** A subagent's own activity (items with `parent_item_id`), by card id. */
export function subagentChildren(items: readonly TurnItem[]): Map<string, TurnItem[]> {
  const out = new Map<string, TurnItem[]>();
  for (const i of items) {
    if (!i.parent_item_id) continue;
    const list = out.get(i.parent_item_id);
    if (list) list.push(i);
    else out.set(i.parent_item_id, [i]);
  }
  return out;
}

/** How a finished turn went, for its status line: `partial` = it answered
 *  but a step failed (Details / Retry), `stopped` = interrupted (neutral, not
 *  an error), `stalled` = the watchdog stopped it. Null while running or
 *  when it simply completed. */
export type TurnOutcome = { kind: 'partial'; failed: TurnItem[]; retryText: string | null } | { kind: 'stopped' | 'stalled' };

export function turnOutcome(p: RuntimeProjection, turn: ProviderTurn | null): TurnOutcome | null {
  if (!turn || turn.status === 'running' || turn.status === 'pending') return null;
  const own = p.items.filter((i) => i.turn_id === turn.id && !i.parent_item_id);
  if (turn.status === 'interrupted' || turn.status === 'cancelled') {
    return { kind: own.some((i) => i.id === `${turn.id}:stalled`) ? 'stalled' : 'stopped' };
  }
  if (turn.status !== 'completed') return null;
  const answered = own.some((i) => i.type === 'assistant_message' && i.text.trim() !== '');
  const failed = own.filter((i) => i.type !== 'system_notice' && !isLostConversation(i) && (i.status === 'failed' || i.type === 'error'));
  if (!answered || failed.length === 0) return null;
  // Retry re-sends the message that started it (none for the agent's own turn).
  const first = own.find((i) => i.type === 'user_message');
  return { kind: 'partial', failed, retryText: first?.type === 'user_message' ? first.text : null };
}

/** The provider no longer has this thread's conversation (Claude's "No
 *  conversation found with session ID …" after a restart). Nothing the
 *  person can act on: the daemon starts a fresh conversation on the next
 *  message and says so in its own notice, so the row is not shown. */
export function isLostConversation(item: TurnItem): boolean {
  return item.type === 'error' && /no conversation found/i.test(item.failure.message);
}

/** An error message without raw session / thread ids (UUIDs and ULIDs). */
export function errorText(message: string): string {
  return message
    .replace(/\s*(?:with )?(?:session|thread)(?: id)?:?\s*[0-9a-f]{8}-[0-9a-f-]{27}/gi, '')
    .replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, '…')
    .trim();
}

/** Tool calls that are plumbing, not work (Claude's deferred-tool lookup). */
const HIDDEN_TOOLS: ReadonlySet<string> = new Set(['ToolSearch']);

/** Otto's own tools in words; anything else from an MCP server reads as
 *  "<tool words> (<server>)". */
const OTTO_TOOL_WORDS: Record<string, string> = {
  routine_create: 'Created a routine',
  routine_update: 'Updated a routine',
  routine_delete: 'Deleted a routine',
  routine_list: 'Checked its routines',
  personality_set: 'Changed its personality',
  instructions_propose: 'Proposed new instructions',
  personal_agent_create: 'Created an agent',
  personal_agent_delegate: 'Handed work to an agent',
  personal_agent_list: 'Checked your agents',
  thread_open: 'Opened a thread',
  thread_message: 'Messaged a thread',
  thread_read: 'Read a thread',
  search_memory: 'Searched its memory',
  send_message: 'Sent a message',
};

/** A tool's name as people say it: `mcp__otto__otto_routine_create` →
 *  "Created a routine", `mcp__github__list_pulls` → "List pulls (github)". */
export function toolWords(name: string): string {
  const m = /^mcp__([^_]+(?:_[^_]+)*?)__(.+)$/.exec(name);
  if (!m) return name;
  const [, server, tool] = m;
  if (server === 'otto') {
    const bare = tool.replace(/^otto_/, '');
    const words = OTTO_TOOL_WORDS[bare] ?? bare.replaceAll('_', ' ');
    return words.charAt(0).toUpperCase() + words.slice(1);
  }
  const words = tool.replaceAll('_', ' ');
  return `${words.charAt(0).toUpperCase()}${words.slice(1)} (${server})`;
}

export function timelineRows(items: readonly TurnItem[]): TimelineRow[] {
  // A subagent's own items render inside its card, not in the timeline;
  // plumbing tool calls and a lost-conversation error not at all.
  items = items.filter((i) => !i.parent_item_id && !isLostConversation(i) && !(i.type === 'dynamic_tool' && HIDDEN_TOOLS.has(i.tool_name)));
  // An approval naming a tool item we have folds into that tool's row.
  const tools = new Set(items.filter(isTool).map((i) => i.id));
  const folded = new Map<string, ApprovalItem>();
  for (const i of items) if (i.type === 'approval_request' && i.tool_item_id && tools.has(i.tool_item_id)) folded.set(i.tool_item_id, i);
  const foldedIds = new Set([...folded.values()].map((a) => a.id));
  const rows: TimelineRow[] = [];
  let work: WorkEntry[] = [];
  let workId = '';
  const flush = (): void => {
    if (work.length) rows.push({ kind: 'work', id: workId, parts: groupParallel(work) });
    work = [];
  };
  for (const item of items) {
    if (item.type === 'session_boundary') {
      flush();
      rows.push({ kind: 'session', id: item.id });
    } else if (item.type === 'user_message' || item.type === 'assistant_message') {
      flush();
      rows.push({ kind: 'message', item });
    } else if (!foldedIds.has(item.id)) {
      if (!work.length) workId = item.id;
      work.push({ item, approval: folded.get(item.id) });
    }
  }
  flush();
  return withDays(rows, items);
}

const itemTime = (i: TurnItem): number => Date.parse(i.started_at ?? i.updated_at);
const dayKey = (ms: number): string => new Date(ms).toDateString();

/** "Today", "Yesterday", "Wed, Nov 4" (a past year adds it: "Mon, Dec 29, 2025"). */
export function dayLabel(ms: number, now: Date = new Date()): string {
  const d = new Date(ms);
  if (d.toDateString() === now.toDateString()) return 'Today';
  const y = new Date(now);
  y.setDate(now.getDate() - 1);
  if (d.toDateString() === y.toDateString()) return 'Yesterday';
  const opts: Intl.DateTimeFormatOptions = { weekday: 'short', month: 'short', day: 'numeric' };
  if (d.getFullYear() !== now.getFullYear()) opts.year = 'numeric';
  return d.toLocaleDateString(undefined, opts);
}

/** Date separators before the first row of each day — only when the thread
 *  spans more than one calendar day (a one-day chat stays uncluttered). */
function withDays(rows: TimelineRow[], items: readonly TurnItem[], now: Date = new Date()): TimelineRow[] {
  const at = new Map(items.map((i) => [i.id, itemTime(i)]));
  const rowTime = (r: TimelineRow): number =>
    (r.kind === 'message' ? at.get(r.item.id) : r.kind === 'work' || r.kind === 'session' ? at.get(r.id) : undefined) ?? NaN;
  const days = new Set(rows.map(rowTime).filter(Number.isFinite).map(dayKey));
  if (days.size < 2) return rows;
  const out: TimelineRow[] = [];
  let last = '';
  for (const r of rows) {
    const t = rowTime(r);
    if (Number.isFinite(t) && dayKey(t) !== last) {
      last = dayKey(t);
      out.push({ kind: 'day', id: `day:${last}`, label: dayLabel(t, now) });
    }
    out.push(r);
  }
  return out;
}

/** Who a framed message came from — an agent's feed turn (`[Message from
 *  Scheduled · Brief (t-1)]: body`) or a linked thread (`[Message from
 *  thread "Fix CI" (claude, id …). Answer it with …]\n\nbody`). Null for a
 *  message the user typed. */
export function messageAttribution(text: string): { from: string; body: string } | null {
  const thread = /^\[(?:Message from|Opened by) thread "([^"]*)"[^\]]*\]\s*\n\n?([\s\S]*)$/.exec(text);
  if (thread) return { from: thread[1] || 'a linked thread', body: thread[2] };
  const feed = /^\[Message from ([^\]]+?)(?: \([^()\]]*\))?\]: ([\s\S]*)$/.exec(text);
  return feed ? { from: feed[1], body: feed[2] } : null;
}

/** The daemon handing a finished routine's report to its agent
 *  (`[Message from Scheduled · Brief (t)]: Otto: "Brief" ran …`). The result
 *  card above already shows the run, and the agent's reply is the brief, so
 *  the chat hides this context message. */
export function isRunHandoff(text: string): boolean {
  const a = messageAttribution(text);
  return !!a && a.body.startsWith('Otto: "');
}

/** T3's `summarizeToolGroup` sentence: "Ran 2 commands and searched the web 3 times". */
export function groupSummary(entries: readonly WorkEntry[]): string {
  const counts = new Map<string, number>();
  for (const { item } of entries) {
    const key =
      item.type === 'web_search' && /fetch/i.test(item.title ?? '') ? 'fetch' : item.type;
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  const labels = [...counts].map(([k, n]) => {
    const s = (one: string, many: string): string => (n === 1 ? one : many);
    switch (k) {
      case 'command_execution':
        return `Ran ${n} ${s('command', 'commands')}`;
      case 'file_change':
        return `Changed ${n} ${s('file', 'files')}`;
      case 'web_search':
        return `Searched the web ${n} ${s('time', 'times')}`;
      case 'fetch':
        return `Fetched ${n} web ${s('page', 'pages')}`;
      default:
        return `Used ${n} ${s('tool', 'tools')}`;
    }
  });
  const parts = labels.map((l, i) => (i === 0 ? l : l.charAt(0).toLowerCase() + l.slice(1)));
  return parts.length < 3 ? parts.join(' and ') : `${parts.slice(0, -1).join(', ')}, and ${parts.at(-1)}`;
}

/** What became of an approval that is no longer pending, or null when there
 *  is nothing worth saying (approved). `agent` names the provider. */
export function approvalOutcome(item: ApprovalItem, request: RuntimeRequest | undefined, agent: string): string | null {
  const reason = item.cancel_reason ?? request?.cancel_reason;
  if (item.status === 'cancelled' || request?.status === 'cancelled') {
    return reason === 'agent'
      ? `Withdrawn by ${agent}`
      : reason === 'interrupt'
        ? 'Canceled when the turn stopped'
        : reason === 'closed'
          ? 'Canceled when the thread closed'
          : 'Canceled';
  }
  if (request?.status === 'expired') return 'Expired';
  if (request?.decision === 'decline' || request?.decision === 'cancel') return 'Declined';
  return null;
}

/** A failed tool's error text (`error` on tool items). */
export function toolError(item: TurnItem): string | null {
  return 'error' in item && typeof item.error === 'string' && item.error ? plainToolError(item.error) : null;
}

/** A tool error in words: a governance refusal (`{"decision":"denied",
 *  "reason":…}`) reads "Not allowed: <reason>", a JSON error its message;
 *  anything else as is (session ids stripped). */
export function plainToolError(raw: string): string {
  const t = raw.trim();
  if (t.startsWith('{')) {
    try {
      const v = JSON.parse(t) as Record<string, unknown>;
      const reason = [v.reason, v.message, v.error].find((x): x is string => typeof x === 'string' && x.trim() !== '');
      if (v.decision === 'denied') return `Not allowed${reason ? `: ${errorText(reason)}` : ''}`;
      if (reason) return errorText(reason);
    } catch {
      /* not JSON — fall through */
    }
  }
  return errorText(t);
}

/** A personal agent's run result posted into its chat: the daemon ends the
 *  notice with ` [otto-run:<id>]` (personal_agent_feed.rs `run_post`) so the
 *  chat can open that run's report. Null for any other notice. */
export interface RunNotice {
  text: string;
  runId: string;
  outcome: 'ok' | 'failed' | 'stopped';
}
export function runNoticeOf(message: string): RunNotice | null {
  const m = /\s*\[otto-run:([A-Za-z0-9_-]+)\]\s*$/.exec(message);
  if (!m) return null;
  const text = message.slice(0, m.index).trim();
  const outcome = /\swas stopped(\s·|$)/.test(text) ? 'stopped' : /\sfailed(\s·|$)/.test(text) ? 'failed' : 'ok';
  return { text, runId: m[1], outcome };
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
      return toolWords(title ?? item.tool_name);
    case 'todo_list': {
      const done = item.steps.filter((s) => s.status === 'completed').length;
      const current = item.steps.find((s) => s.status === 'running') ?? item.steps.find((s) => s.status === 'pending');
      return `Tasks ${done}/${item.steps.length}${current ? ` · ${current.text}` : ''}`;
    }
    case 'approval_request':
      return title ?? item.prompt ?? 'Approval requested';
    case 'system_notice':
      // A run's notice quotes its result's first line, which is often a heading.
      return (runNoticeOf(item.message)?.text ?? item.message).replace(/(^|\s)#{1,6}\s+/g, '$1');
    case 'subagent':
      return title ?? item.description ?? item.subagent_type ?? 'Subagent';
    case 'error':
      return item.failure.class === 'usage_limit' ? 'Usage limit reached' : title || 'Provider error';
    default:
      return title ?? item.type.replaceAll('_', ' ');
  }
}

/** What an Otto session/thread tool call points at, for the quick link under
 *  its row: the thread `otto.thread_open` opened (or `open_session`, which a
 *  thread's agent is given as a linked thread), the peer a
 *  `thread_message` / `thread_read` spoke to, or the terminal session an
 *  `open_session` from elsewhere started. Null for anything else. */
export interface OpenedTarget {
  kind: 'thread' | 'session';
  id: string;
}

const ULID = /"id"\s*:\s*"([0-9A-HJKMNP-TV-Z]{26})"/;

export function openedTarget(item: TurnItem): OpenedTarget | null {
  if (item.type !== 'dynamic_tool' || item.error) return null;
  const tool = item.tool_name.replace(/^.*otto[._]/, '');
  const input = (item.input && typeof item.input === 'object' ? item.input : {}) as Record<string, unknown>;
  const str = (v: unknown): string | null => (typeof v === 'string' && v ? v : null);
  if (tool === 'thread_message') return idOr(str(input.to_session_id), 'thread');
  if (tool === 'thread_read') return idOr(str(input.peer_session_id), 'thread');
  if (tool !== 'thread_open' && tool !== 'open_session') return null;
  const id = item.output?.match(ULID)?.[1] ?? null;
  // A runtime thread's row carries `access_mode`; a terminal session's doesn't.
  const kind = tool === 'thread_open' || item.output?.includes('"access_mode"') ? 'thread' : 'session';
  return idOr(id, kind);
}

function idOr(id: string | null, kind: OpenedTarget['kind']): OpenedTarget | null {
  return id ? { kind, id } : null;
}
