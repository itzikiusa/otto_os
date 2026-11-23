// Threads inside a thread (docs/features/threads-structured-runtime.md
// "Threads inside a thread"): what the parent's timeline and the sidebar
// read about a child thread — the daemon's notice markers, the child's
// state in words, its latest activity while it runs and its answer once it
// ends. After T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.)
// apps/web/src/components/chat/V2LifecycleRow.tsx (`SubagentTimelineLink`:
// progress while running, the result once settled). Pure
// (unit/threadsChildren.test.ts).
import type { ChildOf, RuntimeSession, TurnItem } from '../../../lib/api/types';
import { threadCall } from './agentRows';
import type { AgentTask, AgentTaskStatus } from './agentTasks';
import type { RuntimeProjection } from './projection';
import { latestTurn, workLabel } from './timeline';

export type ChildEvent = 'started' | 'finished' | 'failed' | 'stopped';

/** A `system_notice` the daemon wrote about child threads. */
export interface ThreadNotice {
  /** The words before the marker ("Codex finished: …"). */
  text: string;
  ids: string[];
  event: ChildEvent;
}

/** `… [otto-thread:<id>[,<id>…]:<event>]` (runtime/children.rs). */
export function threadNoticeOf(message: string): ThreadNotice | null {
  const m = /\s*\[otto-thread:([A-Za-z0-9_,-]+):(started|finished|failed|stopped)\]\s*$/.exec(message);
  if (!m) return null;
  const ids = m[1].split(',').filter(Boolean);
  return ids.length ? { text: message.slice(0, m.index).trim(), ids, event: m[2] as ChildEvent } : null;
}

/** `meta.child_of`, when the thread was opened from another one. */
export function childOf(s: RuntimeSession | null | undefined): ChildOf | null {
  const v = s?.meta?.child_of as Partial<ChildOf> | undefined;
  return v && typeof v.id === 'string' && (v.by === 'person' || v.by === 'agent') ? { id: v.id, by: v.by } : null;
}

/** `background`: no turn runs, but work the child's agent started does — its
 *  Stop stops that work (an interrupt would do nothing). */
export type ChildTone = 'starting' | 'working' | 'background' | 'needs-you' | 'done' | 'failed' | 'stopped';

export interface ChildState {
  tone: ChildTone;
  label: string;
}

/** One vocabulary for every subagent: Working · Needs you · Done · Failed · Stopped. */
const LABEL: Record<ChildTone, string> = {
  starting: 'Working',
  working: 'Working',
  background: 'Working',
  'needs-you': 'Needs you',
  done: 'Done',
  failed: 'Failed',
  stopped: 'Stopped',
};

const state = (tone: ChildTone): ChildState => ({ tone, label: LABEL[tone] });

/** The child's state in one word: a waiting approval outranks work, work
 *  outranks how its last turn ended. */
export function childState(s: RuntimeSession | null | undefined, p: RuntimeProjection | null | undefined): ChildState {
  if (!s) return state('starting');
  if (s.status === 'closed') return state('stopped');
  if (s.status === 'awaiting_permission' || p?.requests.some((r) => r.status === 'pending')) return state('needs-you');
  if (s.status === 'starting') return state('starting');
  const turn = p ? latestTurn(p) : null;
  if (s.status === 'working' || turn?.status === 'running' || turn?.status === 'pending') return state('working');
  if ((s.background_tasks?.length ?? 0) > 0) return state('background');
  // No projection loaded: the row says how its last turn ended (newer daemons).
  const ended = turn?.status ?? s.last_turn_status ?? null;
  if (ended === 'failed') return state('failed');
  if (ended === 'interrupted' || ended === 'cancelled') return state('stopped');
  if (ended === 'completed' || answerOf(p) !== '') return state('done');
  return state(s.status === 'dormant' ? 'stopped' : 'starting');
}

/** How a child's latest turn ended: its loaded projection's latest turn,
 *  else the row's `last_turn_status`; null while one runs or when unknown. */
export function turnOutcome(s: RuntimeSession, p: RuntimeProjection | null | undefined): RuntimeSession['last_turn_status'] {
  const t = p ? latestTurn(p) : null;
  if (!t) return s.last_turn_status ?? null;
  if (t.status === 'failed') return 'failed';
  if (t.status === 'interrupted' || t.status === 'cancelled') return 'interrupted';
  if (t.status === 'completed') return 'completed';
  return null;
}

/** The tone RuntimeAgentRow draws. */
export function rowTone(t: ChildTone): 'live' | 'done' | 'failed' | 'off' | 'ask' {
  if (t === 'working' || t === 'starting' || t === 'background') return 'live';
  if (t === 'needs-you') return 'ask';
  if (t === 'done') return 'done';
  if (t === 'failed') return 'failed';
  return 'off';
}

const top = (p: RuntimeProjection): TurnItem[] => p.items.filter((i) => !i.parent_item_id);

/** The child's last answer: the assistant messages of its latest turn that
 *  has any (the whole projection when it has no turns yet). */
export function answerOf(p: RuntimeProjection | null | undefined): string {
  if (!p) return '';
  const said = (list: TurnItem[]): string =>
    list
      .flatMap((i) => (i.type === 'assistant_message' && i.text.trim() ? [i.text.trim()] : []))
      .join('\n\n');
  const turns = [...p.turns].sort((a, b) => b.ordinal - a.ordinal);
  for (const t of turns) {
    const text = said(top(p).filter((i) => i.turn_id === t.id));
    if (text) return text;
  }
  // No turn entities (an old thread's snapshot): the last answer said.
  const last = top(p).findLast((i) => i.type === 'assistant_message' && i.text.trim() !== '');
  return last?.type === 'assistant_message' ? last.text.trim() : '';
}

/** One line of `text` (its first non-empty one, without a leading heading mark). */
export function firstLine(text: string): string {
  const line = text.split('\n').map((l) => l.trim()).find((l) => l !== '') ?? '';
  return line.replace(/^#{1,6}\s+/, '');
}

/** What the child does right now: its newest top-level step's label. */
export function activityOf(p: RuntimeProjection | null | undefined): string | null {
  if (!p) return null;
  const last = top(p).findLast((i) => i.type !== 'user_message');
  if (!last) return null;
  if (last.type === 'assistant_message') return firstLine(last.text) || null;
  return workLabel(last);
}

/** Wall time of the child's work: its first turn's start to its latest
 *  turn's end (or `nowMs` while one runs). */
export function elapsedMs(p: RuntimeProjection | null | undefined, nowMs: number): number | null {
  if (!p || p.turns.length === 0) return null;
  const starts = p.turns.map((t) => Date.parse(t.started_at ?? '')).filter(Number.isFinite);
  if (starts.length === 0) return null;
  const turn = latestTurn(p);
  const live = turn?.status === 'running' || turn?.status === 'pending';
  const end = live ? nowMs : Date.parse(turn?.completed_at ?? '');
  const ms = (Number.isFinite(end) ? end : nowMs) - Math.min(...starts);
  return ms >= 0 ? ms : null;
}

/** Threads opened FROM `parentId` (by a person or its agent), oldest first. */
export function childrenOf(rows: readonly RuntimeSession[], parentId: string): RuntimeSession[] {
  return rows.filter((r) => childOf(r)?.id === parentId).sort((a, b) => a.created_at.localeCompare(b.created_at));
}

type SubagentItem = Extract<TurnItem, { type: 'subagent' }>;
type Hint = { provider?: string | null; title?: string | null; model?: string | null; effort?: string | null };

/** One subagent under a turn: a thread opened from this one (by its agent or
 *  a person), or the provider's own subagent (Claude's Task, …). */
export type Kid = { kind: 'thread'; id: string; hint?: Hint } | { kind: 'native'; item: SubagentItem };

export const kidKey = (k: Kid): string => (k.kind === 'thread' ? k.id : k.item.id);

/** Every turn's subagents as ONE timeline row (T3 Code's V2SubagentGroup):
 *  `groups` = the row at the turn's first subagent item, `skip` = that
 *  turn's other subagent items (drawn inside the row). Sources: the agent's
 *  settled `otto.thread_open` calls, a person's "Delegate" notice and the
 *  provider's own `subagent` cards. */
export function turnSubagents(items: readonly TurnItem[]): { groups: Map<string, Kid[]>; skip: Set<string> } {
  const groups = new Map<string, Kid[]>();
  const skip = new Set<string>();
  const anchorOf = new Map<string, string>();
  const seen = new Set<string>();
  for (const i of items) {
    if (i.parent_item_id) continue;
    let kids: Kid[] = [];
    if (i.type === 'subagent') kids = [{ kind: 'native', item: i }];
    else if (i.type === 'system_notice') {
      const n = threadNoticeOf(i.message);
      // A start, or an end whose start is not in this timeline (paged out).
      if (n) kids = n.ids.map((id) => ({ kind: 'thread', id }));
    } else if (i.type === 'dynamic_tool' && !i.error && i.status !== 'running' && i.status !== 'pending') {
      const c = threadCall(i);
      if (c?.verb === 'Opened' && c.targetId) kids = [{ kind: 'thread', id: c.targetId, hint: { provider: c.provider, title: c.title, model: c.model, effort: c.effort } }];
    }
    kids = kids.filter((k) => !seen.has(kidKey(k)));
    if (kids.length === 0) continue;
    for (const k of kids) seen.add(kidKey(k));
    const turn = i.turn_id ?? `item:${i.id}`;
    const anchor = anchorOf.get(turn);
    if (anchor) {
      groups.get(anchor)!.push(...kids);
      skip.add(i.id);
    } else {
      anchorOf.set(turn, i.id);
      groups.set(i.id, kids);
    }
  }
  return { groups, skip };
}

/** An item `turnSubagents` reads: a subagent card, a subagent notice, an
 *  agent's thread_open call. Anything else (a streaming reply, a command)
 *  never changes the subagent rows. */
export function isSubagentItem(i: TurnItem): boolean {
  if (i.parent_item_id) return false;
  if (i.type === 'subagent') return true;
  if (i.type === 'system_notice') return threadNoticeOf(i.message) !== null;
  return i.type === 'dynamic_tool' && /otto[._]thread_open$/.test(i.tool_name);
}

/** [`turnSubagents`] that runs again only when a subagent item changed (the
 *  same item objects in, the same result out): a long answer streaming in
 *  the timeline re-renders no subagent row. `runs` counts the real runs. */
export function memoTurnSubagents(): { get: (items: readonly TurnItem[]) => ReturnType<typeof turnSubagents>; runs: () => number } {
  let last: TurnItem[] = [];
  let result = turnSubagents([]);
  let runs = 0;
  return {
    get(items) {
      const rel = items.filter(isSubagentItem);
      if (rel.length === last.length && rel.every((x, k) => x === last[k])) return result;
      last = rel;
      runs += 1;
      result = turnSubagents(rel);
      return result;
    },
    runs: () => runs,
  };
}

/** A provider subagent's state in the same words as a thread's. */
export function nativeTone(item: SubagentItem): ChildTone {
  if (item.status === 'running' || item.status === 'pending' || item.status === 'waiting') return 'working';
  if (item.status === 'completed') return 'done';
  return item.status === 'failed' ? 'failed' : 'stopped';
}

/** "2 working · 1 done": the non-zero states, most urgent first. */
export function subagentTally(tones: readonly ChildTone[]): { text: string; tone: 'live' | 'ask' | 'failed' | 'done' } {
  const n = (t: ChildTone[]): number => tones.filter((x) => t.includes(x)).length;
  const parts: [number, string][] = [
    [n(['needs-you']), 'need you'],
    [n(['working', 'starting', 'background']), 'working'],
    [n(['done']), 'done'],
    [n(['failed']), 'failed'],
    [n(['stopped']), 'stopped'],
  ];
  const text = parts
    .filter(([c]) => c > 0)
    .map(([c, w]) => `${c} ${w === 'need you' && c === 1 ? 'needs you' : w}`)
    .join(' · ');
  const tone = parts[0][0] > 0 ? 'ask' : parts[1][0] > 0 ? 'live' : parts[3][0] > 0 ? 'failed' : 'done';
  return { text, tone };
}

// ---- Thread details' "Subagents" section and the sidebar's roll-up ----
// Everything working under a thread — its subagents (a turn's own, threads
// opened from it) and the terminal threads it delegated to — in ONE set of
// status words, plus the thread it works under when it is itself a subagent.
// Commands and timers are not subagents (Background tasks). The one source
// the details card and the sidebar read (unit/threadsSubagents.test.ts).

export type SubagentWord = 'Working' | 'Needs you' | 'Done' | 'Failed' | 'Stopped';

export interface SubagentRow {
  id: string;
  title: string;
  /** Provider slug for the glyph (null for a turn's own subagent). */
  provider: string | null;
  word: SubagentWord;
  startedAt: string | null;
  endedAt: string | null;
  /** The subagent's own thread, when it has one (opens it). */
  threadId: string | null;
  /** Its task row (per-row Stop), when it has one. */
  task: AgentTask | null;
}

export interface SubagentSection {
  /** The thread this one works under. */
  parent: { id: string; title: string } | null;
  running: SubagentRow[];
  /** Finished, newest first (folded under "Previous"). */
  previous: SubagentRow[];
}

/** A delegated terminal thread (or any thread known only by its status). */
export interface SubagentThread {
  id: string;
  title: string;
  provider: string;
  status: string;
}

const WORD: Record<AgentTaskStatus, SubagentWord> = {
  running: 'Working',
  'needs-you': 'Needs you',
  done: 'Done',
  failed: 'Failed',
  stopped: 'Stopped',
};

/** A thread's raw status (runtime or terminal) in the subagent words. */
export function subagentWord(status: string): SubagentWord {
  switch (status) {
    case 'working':
    case 'starting':
    case 'running':
      return 'Working';
    case 'awaiting_permission':
    case 'needs_input':
      return 'Needs you';
    case 'failed':
      return 'Failed';
    case 'closed':
    case 'exited':
      return 'Stopped';
    default:
      return 'Done';
  }
}

const isSubagent = (t: AgentTask): boolean => t.kind === 'agent' || t.childSessionId !== null;
const live = (w: SubagentWord): boolean => w === 'Working' || w === 'Needs you';

export function subagentSection(
  tasks: { running: readonly AgentTask[]; finished: readonly AgentTask[] },
  more: { parent?: { id: string; title: string } | null; delegated?: readonly SubagentThread[]; providerOf?: (threadId: string) => string | null },
): SubagentSection {
  const row = (t: AgentTask): SubagentRow => ({
    id: t.id,
    title: t.label,
    provider: t.childSessionId ? (more.providerOf?.(t.childSessionId) ?? null) : null,
    word: WORD[t.status],
    startedAt: t.startedAt,
    endedAt: t.endedAt,
    threadId: t.childSessionId,
    task: t,
  });
  const seen = new Set<string>();
  const running: SubagentRow[] = [];
  const previous: SubagentRow[] = [];
  const add = (r: SubagentRow): void => {
    const key = r.threadId ?? r.id;
    if (seen.has(key)) return;
    seen.add(key);
    (live(r.word) ? running : previous).push(r);
  };
  for (const t of tasks.running) if (isSubagent(t)) add(row(t));
  for (const t of tasks.finished) if (isSubagent(t)) add(row(t));
  for (const d of more.delegated ?? []) {
    add({ id: d.id, title: d.title, provider: d.provider, word: subagentWord(d.status), startedAt: null, endedAt: null, threadId: d.id, task: null });
  }
  return { parent: more.parent ?? null, running, previous };
}

/** A parent's subagents rolled up for its sidebar row (and a phone): how
 *  many, how many work / need the person / failed, and the first one that
 *  needs the person (its title names it). */
export interface SubagentRollup {
  total: number;
  working: number;
  needsYou: number;
  failed: number;
  needing: string | null;
}

export function childRollup(subs: readonly { title: string; word: SubagentWord }[]): SubagentRollup {
  const r: SubagentRollup = { total: subs.length, working: 0, needsYou: 0, failed: 0, needing: null };
  for (const s of subs) {
    if (s.word === 'Needs you') {
      r.needsYou++;
      r.needing ??= s.title;
    } else if (s.word === 'Working') r.working++;
    else if (s.word === 'Failed') r.failed++;
  }
  return r;
}
