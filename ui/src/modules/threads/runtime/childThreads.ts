// Threads inside a thread (docs/features/threads-structured-runtime.md
// "Threads inside a thread"): what the parent's timeline and the sidebar
// read about a child thread — the daemon's notice markers, the child's
// state in words, its latest activity while it runs and its answer once it
// ends. After T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.)
// apps/web/src/components/chat/V2LifecycleRow.tsx (`SubagentTimelineLink`:
// progress while running, the result once settled). Pure
// (unit/threadsChildren.test.ts).
import type { ChildOf, RuntimeSession, TurnItem } from '../../../lib/api/types';
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

export type ChildTone = 'starting' | 'working' | 'needs-you' | 'done' | 'failed' | 'stopped';

export interface ChildState {
  tone: ChildTone;
  label: string;
}

const LABEL: Record<ChildTone, string> = {
  starting: 'Starting',
  working: 'Working',
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
  if (s.status === 'working' || turn?.status === 'running' || turn?.status === 'pending' || (s.background_tasks?.length ?? 0) > 0) {
    return state('working');
  }
  if (turn?.status === 'failed') return state('failed');
  if (turn?.status === 'interrupted' || turn?.status === 'cancelled') return state('stopped');
  if (turn?.status === 'completed' || answerOf(p) !== '') return state('done');
  return state(s.status === 'dormant' ? 'stopped' : 'starting');
}

/** The tone RuntimeAgentRow draws. */
export function rowTone(t: ChildTone): 'live' | 'done' | 'failed' | 'off' | 'ask' {
  if (t === 'working' || t === 'starting') return 'live';
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
