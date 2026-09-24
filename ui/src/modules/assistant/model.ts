// Pure helpers for the Assistant module — no Svelte, no fetch — so they are
// node-testable (ui/unit/assistantModel.test.ts): routing-hint parsing, the
// provider/model labels, the thread → Spaces/Recent grouping, the card mapping
// (thread turns + tasks → chat cards) and the chat timeline, the needs-you
// reducer with its stale-frame guard, task grouping, and the routing rows.
import type {
  AssistantMemoryChip,
  AssistantRouteKind,
  AssistantTask,
  AssistantTaskState,
  AssistantThread,
  AssistantTurn,
} from '../../lib/api/types';

export type Provider = 'claude' | 'codex';

// ── routing hints ────────────────────────────────────────────────────────────

export interface RouteHint {
  /** The provider a LEADING `@claude` / `@codex` routes this one turn to. */
  provider: Provider | null;
  /** The text the daemon will paste (the leading mention stripped). */
  text: string;
}

// Mirrors the daemon's rule (api.md "Routing"): only a LEADING mention routes,
// as a standalone word — `me@codex.dev`, `@claudette` or a mid-sentence
// "ask @codex" never do.
const LEADING_HINT = /^\s*@(claude|codex)(?=$|[\s.,;:!?])[\s,:;]*/i;

export function parseRouteHint(raw: string): RouteHint {
  const m = LEADING_HINT.exec(raw);
  if (!m) return { provider: null, text: raw };
  return { provider: m[1].toLowerCase() as Provider, text: raw.slice(m[0].length) };
}

// ── labels ───────────────────────────────────────────────────────────────────

export const PROVIDER_NAME: Record<Provider, string> = { claude: 'Claude', codex: 'Codex' };

export function providerName(p: string | null | undefined): string {
  if (!p) return 'Auto';
  return PROVIDER_NAME[p as Provider] ?? p.charAt(0).toUpperCase() + p.slice(1);
}

/** "claude-sonnet-4-5" → "Sonnet", "opus" → "Opus", "gpt-5-codex" → "gpt-5-codex". */
export function modelShort(model: string | null | undefined): string {
  if (!model) return '';
  const m = model.trim();
  const fam = /(opus|sonnet|haiku)/i.exec(m);
  if (fam) return fam[1].charAt(0).toUpperCase() + fam[1].slice(1).toLowerCase();
  return m;
}

/** The chip/badge text: "Claude · Sonnet", "Codex". */
export function providerLabel(p: string | null | undefined, model: string | null | undefined): string {
  const short = modelShort(model);
  return short ? `${providerName(p)} · ${short}` : providerName(p);
}

// ── threads: Spaces 01–04 + Recent ───────────────────────────────────────────

export const SPACE_SLOTS = [1, 2, 3, 4] as const;

export interface ThreadGroups<T> {
  /** Index 0–3 = Space 01–04; null for an unused slot. */
  spaces: (T | null)[];
  /** Everything else, most recently updated first. */
  recent: T[];
}

type ThreadLike = Pick<AssistantThread, 'id' | 'updated_at'> & { space_slot: number | null };

export function groupThreads<T extends ThreadLike>(threads: T[]): ThreadGroups<T> {
  const spaces: (T | null)[] = [null, null, null, null];
  const recent: T[] = [];
  for (const t of threads) {
    const slot = t.space_slot;
    if (slot != null && slot >= 1 && slot <= 4 && spaces[slot - 1] == null) spaces[slot - 1] = t;
    else recent.push(t);
  }
  recent.sort((a, b) => cmpDesc(a.updated_at, b.updated_at));
  return { spaces, recent };
}

/** The thread the page opens on: the remembered one if it still exists, else the most recently updated. */
export function latestThreadId<T extends Pick<AssistantThread, 'id' | 'updated_at'>>(threads: T[], remembered: string | null): string | null {
  if (remembered && threads.some((t) => t.id === remembered)) return remembered;
  let best: T | null = null;
  for (const t of threads) if (!best || cmpDesc(t.updated_at, best.updated_at) < 0) best = t;
  return best?.id ?? null;
}

export function spaceLabel(slot: number): string {
  return String(slot).padStart(2, '0');
}

// ── card mapping: thread turns + tasks → chat cards ──────────────────────────

/** Everything in a thread that is not a message bubble. */
export type ChatCard =
  /** A `memory` turn — rendered as chips on the reply it belongs to. */
  | { kind: 'memory'; id: string; created_at: string; turn: AssistantTurn; chip: AssistantMemoryChip | null }
  /** A `task` / `reminder` / `approval` / `delegation` turn, bound to its live task. */
  | { kind: 'task'; id: string; created_at: string; turn: AssistantTurn | null; task_id: string }
  /** A `route` / `limit` system line (or a system message). */
  | { kind: 'line'; id: string; created_at: string; turn: AssistantTurn };

const TASK_TURN_KINDS = new Set<AssistantTurn['kind']>(['task', 'reminder', 'approval', 'delegation']);

export function taskIdOf(turn: Pick<AssistantTurn, 'data'>): string | null {
  const v = turn.data?.task_id;
  return typeof v === 'string' && v ? v : null;
}

export function memoryChipOf(turn: Pick<AssistantTurn, 'kind' | 'data'>): AssistantMemoryChip | null {
  if (turn.kind !== 'memory' || !turn.data) return null;
  const d = turn.data as Partial<AssistantMemoryChip>;
  if (d.action !== 'remembered' && d.action !== 'forgot' && d.action !== 'pending') return null;
  return { action: d.action, memory_ids: Array.isArray(d.memory_ids) ? d.memory_ids : [], undo: d.undo ?? null };
}

/** Message turns (bubbles) — the thread index used until the CLI transcript exists. */
export function messageTurns(turns: AssistantTurn[]): AssistantTurn[] {
  return turns.filter((t) => t.kind === 'message' && t.role !== 'system');
}

/**
 * Map a thread's non-message turns and its tasks to cards. A task referenced
 * by a turn renders at that turn; a task of this thread no turn mentions yet
 * (an approval or a reminder the agent just opened) renders at its own
 * `created_at`, so nothing the assistant is doing is ever invisible.
 */
export function threadCards(turns: AssistantTurn[], tasks: AssistantTask[], threadId: string): ChatCard[] {
  const cards: ChatCard[] = [];
  const seen = new Set<string>();
  for (const t of turns) {
    if (t.kind === 'message') {
      if (t.role === 'system') cards.push({ kind: 'line', id: t.id, created_at: t.created_at, turn: t });
      continue;
    }
    if (t.kind === 'memory') {
      cards.push({ kind: 'memory', id: t.id, created_at: t.created_at, turn: t, chip: memoryChipOf(t) });
      continue;
    }
    const taskId = taskIdOf(t);
    if (TASK_TURN_KINDS.has(t.kind) && taskId) {
      // One card per task: a later turn about the same task (the reminder
      // firing, the delegation's result) doesn't duplicate the card.
      if (seen.has(taskId)) continue;
      seen.add(taskId);
      cards.push({ kind: 'task', id: t.id, created_at: t.created_at, turn: t, task_id: taskId });
    } else {
      if (t.kind === 'limit' && taskId) seen.add(taskId);
      cards.push({ kind: 'line', id: t.id, created_at: t.created_at, turn: t });
    }
  }
  for (const task of tasks) {
    if (task.thread_id !== threadId || seen.has(task.id)) continue;
    // Memory reviews live on the Memory tab; the rest belong in the thread.
    if (task.kind === 'memory_review') continue;
    cards.push({ kind: 'task', id: `task:${task.id}`, created_at: task.created_at, turn: null, task_id: task.id });
  }
  return cards;
}

// ── the chat timeline ────────────────────────────────────────────────────────

/** Anything in the transcript a card can sit next to: a user bubble or a response. */
export interface TimelineTurn {
  id: string;
  role: 'user' | 'assistant';
  ts: string | null;
}

export type TimelineEntry<T extends TimelineTurn> =
  | { kind: 'turn'; turn: T; memory: Extract<ChatCard, { kind: 'memory' }>[] }
  | { kind: 'card'; card: ChatCard };

/**
 * Interleave cards with the messages by time. Memory chips attach to the
 * first response at or after them (the reply that remembered it); if that
 * reply hasn't landed yet they show on their own row at their time.
 */
export function buildTimeline<T extends TimelineTurn>(turns: T[], cards: ChatCard[]): TimelineEntry<T>[] {
  const memBy = new Map<string, Extract<ChatCard, { kind: 'memory' }>[]>();
  const loose: ChatCard[] = [];
  for (const c of cards) {
    if (c.kind === 'memory') {
      const anchor = firstResponseAtOrAfter(turns, c.created_at);
      if (anchor) {
        const list = memBy.get(anchor) ?? [];
        list.push(c);
        memBy.set(anchor, list);
        continue;
      }
    }
    loose.push(c);
  }
  loose.sort((a, b) => cmpAsc(a.created_at, b.created_at));
  const out: TimelineEntry<T>[] = [];
  let li = 0;
  for (const t of turns) {
    // A card lands before the first message that is strictly later than it.
    while (li < loose.length && t.ts != null && cmpAsc(loose[li].created_at, t.ts) < 0) out.push({ kind: 'card', card: loose[li++] });
    out.push({ kind: 'turn', turn: t, memory: memBy.get(t.id) ?? [] });
  }
  while (li < loose.length) out.push({ kind: 'card', card: loose[li++] });
  return out;
}

function firstResponseAtOrAfter(turns: TimelineTurn[], ts: string): string | null {
  for (const t of turns) if (t.role === 'assistant' && t.ts != null && cmpAsc(t.ts, ts) >= 0) return t.id;
  return null;
}

// ── needs you ────────────────────────────────────────────────────────────────

export interface NeedsYouState {
  /** Tasks in state `needs_you`, oldest first. */
  items: AssistantTask[];
  /** id → the newest `updated_at` seen for that task (from any source). */
  seen: Record<string, string>;
}

export type NeedsYouAction =
  /** A GET snapshot; `startedAt` = when the request was sent. */
  | { type: 'load'; items: AssistantTask[]; startedAt: string }
  /** A live frame or an action's response: the task row after the change. */
  | { type: 'upsert'; task: AssistantTask };

/**
 * The needs-you reducer. A task is in the queue iff its newest known row is in
 * state `needs_you`. Rows older than what we've seen are ignored — WS frames,
 * action responses and a slow GET can arrive in any order — and a snapshot
 * can't drop an item that a frame added after the request went out.
 */
export function reduceNeedsYou(state: NeedsYouState, action: NeedsYouAction): NeedsYouState {
  if (action.type === 'upsert') {
    const t = action.task;
    const have = state.seen[t.id];
    if (have && cmpAsc(t.updated_at, have) < 0) return state;
    const seen = { ...state.seen, [t.id]: t.updated_at };
    const rest = state.items.filter((i) => i.id !== t.id);
    return { items: t.state === 'needs_you' ? sortNeeds([...rest, t]) : rest, seen };
  }
  const seen = { ...state.seen };
  const queue = new Map(state.items.map((i) => [i.id, i] as const));
  const inSnapshot = new Set<string>();
  for (const t of action.items) {
    inSnapshot.add(t.id);
    const have = seen[t.id];
    if (have && cmpAsc(t.updated_at, have) < 0) continue; // we already know a newer row
    seen[t.id] = t.updated_at;
    if (t.state === 'needs_you') queue.set(t.id, t);
    else queue.delete(t.id);
  }
  // An item the snapshot omits left the queue — unless it changed after the
  // request went out (a frame we applied while the GET was in flight).
  for (const i of state.items) {
    if (inSnapshot.has(i.id)) continue;
    if (cmpAsc(seen[i.id] ?? i.updated_at, action.startedAt) < 0) queue.delete(i.id);
  }
  return { items: sortNeeds([...queue.values()]), seen };
}

export function needsYouByThread(items: Pick<AssistantTask, 'thread_id'>[]): Record<string, number> {
  const out: Record<string, number> = {};
  for (const i of items) if (i.thread_id) out[i.thread_id] = (out[i.thread_id] ?? 0) + 1;
  return out;
}

function sortNeeds(items: AssistantTask[]): AssistantTask[] {
  return [...items].sort((a, b) => cmpAsc(a.created_at, b.created_at));
}

// ── tasks ────────────────────────────────────────────────────────────────────

export type Tone = 'ok' | 'warn' | 'bad' | 'info' | 'neutral';

export const TASK_STATE: Record<AssistantTaskState, { label: string; tone: Tone }> = {
  needs_you: { label: 'Needs you', tone: 'warn' },
  running: { label: 'Running', tone: 'info' },
  queued: { label: 'Queued', tone: 'neutral' },
  done: { label: 'Done', tone: 'ok' },
  failed: { label: 'Failed', tone: 'bad' },
  cancelled: { label: 'Cancelled', tone: 'neutral' },
};

export const TASK_KIND: Record<AssistantTask['kind'], string> = {
  task: 'Task',
  reminder: 'Reminder',
  approval: 'Approval',
  question: 'Question',
  takeover: 'Take over',
  limit: 'Usage limit',
  delegation: 'Delegated',
  memory_review: 'Memory review',
};

/** A reminder reads "Scheduled" / "Delivered" rather than queued / done. */
export function taskStateLabel(t: Pick<AssistantTask, 'state' | 'kind'>): string {
  if (t.kind === 'reminder') {
    if (t.state === 'queued') return 'Scheduled';
    if (t.state === 'done') return 'Delivered';
  }
  return TASK_STATE[t.state]?.label ?? t.state;
}

export function taskTone(t: Pick<AssistantTask, 'state' | 'kind'>): Tone {
  if (t.kind === 'reminder' && t.state === 'queued') return 'ok';
  return TASK_STATE[t.state]?.tone ?? 'neutral';
}

export type TaskColumn = 'needs_you' | 'running' | 'queued' | 'finished';

/** Tasks by board column (failed/cancelled sit with done); newest change first. */
export function groupTasks<T extends Pick<AssistantTask, 'state' | 'updated_at'>>(tasks: T[]): Record<TaskColumn, T[]> {
  const out: Record<TaskColumn, T[]> = { needs_you: [], running: [], queued: [], finished: [] };
  for (const t of tasks) {
    const col: TaskColumn = t.state === 'needs_you' || t.state === 'running' || t.state === 'queued' ? t.state : 'finished';
    out[col].push(t);
  }
  for (const k of Object.keys(out) as TaskColumn[]) out[k].sort((a, b) => cmpDesc(a.updated_at, b.updated_at));
  return out;
}

/** Upsert a row unless it is older than the copy we hold. New rows go first (newest-first lists). */
export function upsertNewer<T extends { id: string; updated_at: string }>(list: T[], next: T): T[] {
  const i = list.findIndex((x) => x.id === next.id);
  if (i < 0) return [next, ...list];
  if (cmpAsc(next.updated_at, list[i].updated_at) < 0) return list;
  const copy = list.slice();
  copy[i] = next;
  return copy;
}

/** Browser progress a task may carry in `result.browser` (UI convention; see the contract gaps). */
export interface BrowserProgress {
  url: string | null;
  tab_id: string | null;
  thumbnail_url: string | null;
  steps: { label: string; state: 'done' | 'current' | 'todo' }[];
  note: string | null;
}

export function browserOf(task: Pick<AssistantTask, 'result'> | null | undefined): BrowserProgress | null {
  const b = task?.result?.browser;
  if (!b || typeof b !== 'object') return null;
  const r = b as Record<string, unknown>;
  const str = (v: unknown): string | null => (typeof v === 'string' && v ? v : null);
  const steps = Array.isArray(r.steps)
    ? r.steps
        .map((s) => (s && typeof s === 'object' ? (s as Record<string, unknown>) : null))
        .filter((s): s is Record<string, unknown> => !!s && typeof s.label === 'string')
        .map((s): BrowserProgress['steps'][number] => ({ label: s.label as string, state: s.state === 'done' ? 'done' : s.state === 'current' ? 'current' : 'todo' }))
    : [];
  return { url: str(r.url), tab_id: str(r.tab_id), thumbnail_url: str(r.thumbnail_url), steps, note: str(r.note) };
}

// ── routing settings ─────────────────────────────────────────────────────────

export const ROUTE_ROWS: { kind: AssistantRouteKind; label: string; hint: string }[] = [
  { kind: 'chat', label: 'Conversation, writing, planning', hint: 'Also research and browser chores' },
  { kind: 'code', label: 'Code, shell, data & files', hint: '“fix this script”, spreadsheets, commands' },
  { kind: 'hard', label: '“Think hard” / long tasks', hint: 'Multi-step or hours-long work' },
  { kind: 'voice', label: 'Voice conversation', hint: 'Latency matters most' },
];

/** "a, b ,, c" → ["a", "b", "c"] (trimmed, lower-cased, de-duplicated). */
export function parseKeywords(raw: string): string[] {
  const out: string[] = [];
  for (const w of raw.split(/[,\n]/)) {
    const k = w.trim().toLowerCase();
    if (k && !out.includes(k)) out.push(k);
  }
  return out;
}

/** Each provider's share of this week's tokens, whole percents summing to 100. */
export function loadShare(rows: { provider: string; total_tokens: number }[]): { provider: string; pct: number; tokens: number }[] {
  const total = rows.reduce((s, r) => s + Math.max(0, r.total_tokens), 0);
  if (total <= 0) return [];
  const raw = rows.filter((r) => r.total_tokens > 0).map((r) => ({ provider: r.provider, tokens: r.total_tokens, exact: (r.total_tokens / total) * 100 }));
  const floored = raw.map((r) => ({ ...r, pct: Math.floor(r.exact) }));
  let left = 100 - floored.reduce((s, r) => s + r.pct, 0);
  for (const r of [...floored].sort((a, b) => b.exact - Math.floor(b.exact) - (a.exact - Math.floor(a.exact)))) {
    if (left <= 0) break;
    r.pct += 1;
    left -= 1;
  }
  return floored.sort((a, b) => b.tokens - a.tokens).map(({ provider, pct, tokens }) => ({ provider, pct, tokens }));
}

// ── misc ─────────────────────────────────────────────────────────────────────

/** "Claude limit reached until 14:00 — continue on Codex?" */
export function limitNotice(provider: string, until: string | null, alternative: string | null, clock: (iso: string) => string): string {
  const at = until ? clock(until) : '';
  const head = `${providerName(provider)} limit reached${at ? ` until ${at}` : ''}`;
  return alternative ? `${head} — continue on ${providerName(alternative)}?` : head;
}

/** Timestamps from the transcript and the daemon differ in precision/offset,
 *  so compare as instants; fall back to string order for unparsable values. */
function cmpAsc(a: string, b: string): number {
  const x = Date.parse(a);
  const y = Date.parse(b);
  if (Number.isFinite(x) && Number.isFinite(y)) return x - y;
  return a < b ? -1 : a > b ? 1 : 0;
}

function cmpDesc(a: string, b: string): number {
  return cmpAsc(b, a);
}
