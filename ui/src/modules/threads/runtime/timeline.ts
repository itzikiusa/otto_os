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

import type {
  ApprovalOption,
  ContextHandoff,
  MessageAttachment,
  ProviderTurn,
  RequestKind,
  RuntimeRequest,
  TurnItem,
} from '../../../lib/api/types';
import type { RuntimeProjection } from './projection';
import { settingsChangeOf, type SettingsChange } from './settingsChange.ts';

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
  /** `handoff`: a provider switch ("Context handoff · Claude Code → Codex"). */
  | { kind: 'session'; id: string; handoff?: ContextHandoff }
  /** Subtle divider before an autonomous/reactive agent wakeup turn (`origin === 'agent'`). */
  | { kind: 'agent_turn'; id: string; label: string }
  /** A quiet line where a model / effort / access switch took effect or was refused (settingsChange.ts). */
  | { kind: 'setting'; id: string; change: SettingsChange };

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

const EMPTY_SUBAGENT_CHILDREN: ReadonlyMap<string, TurnItem[]> = new Map();

/** A subagent's own activity (items with `parent_item_id`), by card id. */
export function subagentChildren(items: readonly TurnItem[]): Map<string, TurnItem[]> {
  let hasParent = false;
  for (let idx = 0; idx < items.length; idx++) {
    if (items[idx].parent_item_id) {
      hasParent = true;
      break;
    }
  }
  if (!hasParent) return EMPTY_SUBAGENT_CHILDREN as Map<string, TurnItem[]>;
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
 *  an error), `stalled` = the watchdog stopped it, `failed` = terminated with
 *  an error. Null while running or when it simply completed. */
export type TurnOutcome =
  | {
      kind: 'partial';
      failed: TurnItem[];
      retryText: string | null;
      retryAttachments?: MessageAttachment[];
    }
  | {
      kind: 'stopped' | 'stalled' | 'failed';
      retryText?: string;
      retryAttachments?: MessageAttachment[];
    };

export function turnOutcome(p: RuntimeProjection, turn: ProviderTurn | null): TurnOutcome | null {
  if (!turn || turn.status === 'running' || turn.status === 'pending') return null;
  const own = p.items.filter((i) => i.turn_id === turn.id && !i.parent_item_id);
  const first = own.find((i): i is Extract<TurnItem, { type: 'user_message' }> => i.type === 'user_message');
  const retryFields = first
    ? {
        ...(first.text ? { retryText: first.text } : {}),
        ...(first.attachments?.length ? { retryAttachments: first.attachments } : {}),
      }
    : {};
  if (turn.status === 'interrupted' || turn.status === 'cancelled') {
    const kind = own.some((i) => i.id === `${turn.id}:stalled`) ? 'stalled' : 'stopped';
    return { kind, ...retryFields };
  }
  if (turn.status === 'failed') {
    return { kind: 'failed', ...retryFields };
  }
  if (turn.status !== 'completed') return null;
  const answered = own.some((i) => i.type === 'assistant_message' && i.text.trim() !== '');
  const failed = own.filter((i) => i.type !== 'system_notice' && !isLostConversation(i) && (i.status === 'failed' || i.type === 'error'));
  if (!answered || failed.length === 0) return null;
  // Retry re-sends the message that started it (none for the agent's own turn).
  return {
    kind: 'partial',
    failed,
    retryText: first ? first.text : null,
    ...(first?.attachments?.length ? { retryAttachments: first.attachments } : {}),
  };
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

type MessageTurnItem = Extract<TurnItem, { type: 'user_message' | 'assistant_message' }>;
type MessageTimelineRow = Extract<TimelineRow, { kind: 'message' }>;
type WorkTimelineRow = Extract<TimelineRow, { kind: 'work' }>;
type SessionTimelineRow = Extract<TimelineRow, { kind: 'session' }>;
type AgentTurnTimelineRow = Extract<TimelineRow, { kind: 'agent_turn' }>;
type DayTimelineRow = Extract<TimelineRow, { kind: 'day' }>;

const messageRowCache = new WeakMap<MessageTurnItem, MessageTimelineRow>();
const workRowCache = new WeakMap<TurnItem, { entries: WorkEntry[]; row: WorkTimelineRow }>();
const workEntriesCache = new WeakMap<WorkTimelineRow, WorkEntry[]>();
const sessionRowCache = new WeakMap<TurnItem, SessionTimelineRow>();
const rowAnchorItem = new WeakMap<TimelineRow, TurnItem>();
const itemDayCache = new WeakMap<TurnItem, { time: number; day: string }>();
const agentTurnRowCache = new Map<string, AgentTurnTimelineRow>();
const dayRowCache = new Map<string, DayTimelineRow>();

/** Flat `WorkEntry[]` for a `work` row, memoized per row object reference. */
export function workRowEntries(row: WorkTimelineRow): WorkEntry[] {
  let flat = workEntriesCache.get(row);
  if (!flat) {
    flat = row.parts.flatMap((part) => (part.kind === 'group' ? part.entries : [part.entry]));
    workEntriesCache.set(row, flat);
  }
  return flat;
}

function getOrCreateWorkRow(workId: string, work: WorkEntry[]): WorkTimelineRow {
  const lead = work[0].item;
  const cached = workRowCache.get(lead);
  if (
    cached &&
    cached.row.id === workId &&
    cached.entries.length === work.length &&
    cached.entries.every((e, i) => e.item === work[i].item && e.approval === work[i].approval)
  ) {
    return cached.row;
  }
  const row: WorkTimelineRow = { kind: 'work', id: workId, parts: groupParallel(work) };
  workEntriesCache.set(row, work);
  rowAnchorItem.set(row, lead);
  workRowCache.set(lead, { entries: work, row });
  return row;
}

export function timelineRows(items: readonly TurnItem[], turns: readonly ProviderTurn[] = []): TimelineRow[] {
  // A subagent's own items render inside its card, not in the timeline;
  // plumbing tool calls and a lost-conversation error not at all.
  items = items.filter((i) => !i.parent_item_id && !isLostConversation(i) && !(i.type === 'dynamic_tool' && HIDDEN_TOOLS.has(i.tool_name)));
  let agentTurnIds: Set<string> | null = null;
  for (let i = 0; i < turns.length; i++) {
    if (turns[i].origin === 'agent') {
      if (!agentTurnIds) agentTurnIds = new Set();
      agentTurnIds.add(turns[i].id);
    }
  }
  const shownAgentTurns = agentTurnIds ? new Set<string>() : null;
  // An approval naming a tool item we have folds into that tool's row.
  let folded: Map<string, ApprovalItem> | null = null;
  let foldedIds: Set<string> | null = null;
  for (let i = 0; i < items.length; i++) {
    if (items[i].type === 'approval_request') {
      const tools = new Set<string>();
      for (let j = 0; j < items.length; j++) if (isTool(items[j])) tools.add(items[j].id);
      folded = new Map();
      foldedIds = new Set();
      for (let j = 0; j < items.length; j++) {
        const cand = items[j];
        if (cand.type === 'approval_request' && cand.tool_item_id && tools.has(cand.tool_item_id)) {
          folded.set(cand.tool_item_id, cand);
          foldedIds.add(cand.id);
        }
      }
      break;
    }
  }
  const rows: TimelineRow[] = [];
  let work: WorkEntry[] = [];
  let workId = '';
  const flush = (): void => {
    if (work.length) rows.push(getOrCreateWorkRow(workId, work));
    work = [];
  };
  for (const item of items) {
    if (item.turn_id && agentTurnIds?.has(item.turn_id) && !shownAgentTurns!.has(item.turn_id) && item.type !== 'session_boundary') {
      shownAgentTurns!.add(item.turn_id);
      flush();
      const divId = `agent-turn:${item.turn_id}`;
      let divRow = agentTurnRowCache.get(divId);
      if (!divRow) {
        if (agentTurnRowCache.size >= 512) agentTurnRowCache.clear();
        divRow = { kind: 'agent_turn', id: divId, label: 'Background update' };
        agentTurnRowCache.set(divId, divRow);
      }
      rows.push(divRow);
    }
    if (item.type === 'session_boundary') {
      flush();
      let sRow = sessionRowCache.get(item);
      if (!sRow) {
        sRow = { kind: 'session', id: item.id, ...(item.handoff ? { handoff: item.handoff } : {}) };
        rowAnchorItem.set(sRow, item);
        sessionRowCache.set(item, sRow);
      }
      rows.push(sRow);
    } else if (item.type === 'user_message' || item.type === 'assistant_message') {
      flush();
      let mRow = messageRowCache.get(item);
      if (!mRow) {
        mRow = { kind: 'message', item };
        rowAnchorItem.set(mRow, item);
        messageRowCache.set(item, mRow);
      }
      rows.push(mRow);
    } else if (settingsChangeOf(item)) {
      flush();
      rows.push({ kind: 'setting', id: item.id, change: settingsChangeOf(item)! });
    } else if (!foldedIds?.has(item.id)) {
      if (!work.length) workId = item.id;
      const approval = folded?.get(item.id);
      work.push({ item, approval });
    }
  }
  flush();
  return withDays(rows, items);
}

const itemTime = (i: TurnItem): number => Date.parse(i.started_at ?? i.updated_at);
const dayKey = (ms: number): string => new Date(ms).toDateString();

function cachedItemDay(i: TurnItem): { time: number; day: string } {
  let hit = itemDayCache.get(i);
  if (!hit) {
    const time = itemTime(i);
    hit = { time, day: Number.isFinite(time) ? dayKey(time) : '' };
    itemDayCache.set(i, hit);
  }
  return hit;
}

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
function withDays(rows: TimelineRow[], _items: readonly TurnItem[], now: Date = new Date()): TimelineRow[] {
  let firstDay = '';
  let multiDay = false;
  for (let i = 0; i < rows.length; i++) {
    const anchor = rowAnchorItem.get(rows[i]);
    if (!anchor) continue;
    const { day } = cachedItemDay(anchor);
    if (!day) continue;
    if (!firstDay) firstDay = day;
    else if (day !== firstDay) {
      multiDay = true;
      break;
    }
  }
  if (!multiDay) return rows;
  const out: TimelineRow[] = [];
  let last = '';
  for (const r of rows) {
    const anchor = rowAnchorItem.get(r);
    if (anchor) {
      const { time, day } = cachedItemDay(anchor);
      if (day && day !== last) {
        last = day;
        const label = dayLabel(time, now);
        const cacheKey = `${last}\0${label}`;
        let dayRow = dayRowCache.get(cacheKey);
        if (!dayRow) {
          if (dayRowCache.size >= 256) dayRowCache.clear();
          dayRow = { kind: 'day', id: `day:${last}`, label };
          dayRowCache.set(cacheKey, dayRow);
        }
        out.push(dayRow);
      }
    }
    out.push(r);
  }
  return out;
}

/** Who a framed message came from — an agent's feed turn (`[Message from
 *  Scheduled · Brief (t-1)]: body`), a linked thread (`[Message from
 *  thread "Fix CI" (claude, id …). Answer it with …]\n\nbody`), or a child
 *  thread's answer woken into its parent (`[Reply from subagent "Fix CI"
 *  (codex, id …): its turn finished. …]\n\nbody`, otto-agent-protocol
 *  runtime/children.rs `wake_text`; `reply: true`; stored history may
 *  still read `[Reply from thread …]` / `[Opened by thread …]`), or a
 *  subagent's task (`[Subagent task from thread "Fix CI" (…). …]`). The header is the whole
 *  first line, so a `]` in a title does not end it early. Null for a message
 *  the user typed. */
export function messageAttribution(text: string): { from: string; body: string; reply?: true; task?: true } | null {
  const thread = /^\[(Message from thread|Opened by thread|Subagent task from thread|Reply from thread|Reply from subagent) "(.*?)" \([^\n]*\]\s*\n\n?([\s\S]*)$/.exec(text);
  if (thread) {
    const kind = thread[1].startsWith('Reply from') ? { reply: true as const } : thread[1] === 'Message from thread' ? {} : { task: true as const };
    return { from: thread[2] || 'the parent thread', body: thread[3], ...kind };
  }
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
        return `Edited ${n} ${s('file', 'files')}`;
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

const SHORT_CLOCK_FMT = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' });

function pad2(n: number): string {
  return String(n).padStart(2, '0');
}

function fmtLocalDate(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}

function fmtLocalClockSec(d: Date): string {
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`;
}

function fmtLocalDateTime(d: Date): string {
  return `${fmtLocalDate(d)} ${fmtLocalClockSec(d)}`;
}

/** "820ms", "14s", "2m 14s", "1h 3m". */
export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return '';
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

/** Local short `HH:mm` clock for message rows, or `""` when invalid/absent. */
export function formatMessageClock(iso: string | null | undefined): string {
  if (!iso) return '';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  return SHORT_CLOCK_FMT.format(d);
}

/** Full local date + seconds tooltip for sent, streaming, and settled messages. */
export function formatMessageTooltip(
  startedAt: string | null | undefined,
  completedAt?: string | null,
  streaming?: boolean,
): string {
  const start = startedAt ? new Date(startedAt) : null;
  const startOk = start !== null && !Number.isNaN(start.getTime());
  const end = completedAt ? new Date(completedAt) : null;
  const endOk = end !== null && !Number.isNaN(end.getTime());

  if (streaming) {
    const d = startOk ? start! : endOk ? end! : null;
    return d ? `Streaming since ${fmtLocalClockSec(d)}` : '';
  }

  if (completedAt !== undefined) {
    if (startOk && endOk) {
      const elapsedMs = end!.getTime() - start!.getTime();
      const span = Number.isFinite(elapsedMs) && elapsedMs >= 1000 ? formatDuration(elapsedMs) : '';
      const suffix = span ? ` (${span})` : '';
      return `Started ${fmtLocalClockSec(start!)} · Completed ${fmtLocalClockSec(end!)}${suffix} · ${fmtLocalDate(end!)}`;
    }
    if (endOk) return `Completed ${fmtLocalDateTime(end!)}`;
    if (startOk) return `Started ${fmtLocalDateTime(start!)}`;
    return '';
  }

  return startOk ? `Sent ${fmtLocalDateTime(start!)}` : '';
}
