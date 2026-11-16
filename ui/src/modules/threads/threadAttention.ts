// Which thread needs you, at a glance: the urgency order every Threads list
// sorts by (needs you › working › failed › everything else), the roll-up a
// folded group or a parent shows for its members, the words over a set of
// child threads, which thread "Next that needs you" opens, and how ↑/↓ move
// through the list. Pure (unit/threadsAttention.test.ts) — states come in
// already derived by lib/status.ts `sessionState`.

/** The part of a derived state these helpers read (`SessionStateInfo`). */
export interface StateLike {
  key: string;
  label: string;
}

/** Lower is more urgent. A background-only row (`running`) counts as work. */
export function urgency(st: StateLike | null | undefined): number {
  switch (st?.key) {
    case 'needs-you':
      return 0;
    case 'working':
    case 'running':
    case 'stale':
      return 1;
    case 'failed':
      return 2;
    default:
      return 3;
  }
}

/** The most urgent of several states (a parent and its children); null for none. */
export function mostUrgent<T extends StateLike>(states: readonly (T | null | undefined)[]): T | null {
  let best: T | null = null;
  for (const st of states) if (st && (!best || urgency(st) < urgency(best))) best = st;
  return best;
}

/** "1 needs you" / "2 need you" (verb agreement, not a noun plural). */
const needVerb = (n: number): string => (n === 1 ? 'needs' : 'need');

export interface Rollup {
  needs: number;
  working: number;
  failed: number;
  total: number;
}

export function rollup(states: readonly (StateLike | null | undefined)[]): Rollup {
  const r: Rollup = { needs: 0, working: 0, failed: 0, total: states.length };
  for (const st of states) {
    const u = urgency(st);
    if (u === 0) r.needs++;
    else if (u === 1) r.working++;
    else if (u === 2) r.failed++;
  }
  return r;
}

/** A folded group header's note: "1 needs you", else "2 failed", else ''. */
export function rollupNote(r: Rollup): { text: string; tone: 'warning' | 'danger' } | null {
  if (r.needs > 0) return { text: `${r.needs} ${needVerb(r.needs)} you`, tone: 'warning' };
  if (r.failed > 0) return { text: `${r.failed} failed`, tone: 'danger' };
  return null;
}

/** The child-thread states RuntimeChildGroup tallies (childThreads.ts `ChildTone`). */
export type ChildToneLike = 'starting' | 'working' | 'background' | 'needs-you' | 'done' | 'failed' | 'stopped';

/** "1 needs you · 1 working", "2 failed", "all answered" — from the real
 *  child states, with the tone of the most pressing one. */
export function childGroupSummary(tones: readonly ChildToneLike[]): { text: string; tone: 'warning' | 'danger' | null } {
  const count = (pred: (t: ChildToneLike) => boolean): number => tones.filter(pred).length;
  const needs = count((t) => t === 'needs-you');
  const working = count((t) => t === 'working' || t === 'starting' || t === 'background');
  const failed = count((t) => t === 'failed');
  const stopped = count((t) => t === 'stopped');
  const parts: string[] = [];
  if (needs) parts.push(`${needs} ${needVerb(needs)} you`);
  if (working) parts.push(`${working} working`);
  if (failed) parts.push(`${failed} failed`);
  if (stopped) parts.push(`${stopped} stopped`);
  const answered = tones.length - needs - working - failed - stopped;
  if (parts.length === 0) return { text: tones.length ? 'all answered' : '', tone: null };
  if (answered > 0) parts.push(`${answered} answered`);
  return { text: parts.join(' · '), tone: needs ? 'warning' : failed ? 'danger' : null };
}

/** The thread after `current` in `order` that needs you, wrapping around
 *  (never `current` itself); null when no other thread does. */
export function nextNeedsYou(order: readonly string[], current: string | null | undefined, needs: (id: string) => boolean): string | null {
  const n = order.length;
  if (n === 0) return null;
  const at = current ? order.indexOf(current) : -1;
  for (let step = 1; step <= n; step++) {
    const id = order[(at + step + n) % n];
    if (id !== current && needs(id)) return id;
  }
  return null;
}

/** The thread `dir` steps from `current` in `order` (clamped, no wrap);
 *  the first / last one when `current` is not listed. */
export function stepThread(order: readonly string[], current: string | null | undefined, dir: 1 | -1): string | null {
  if (order.length === 0) return null;
  const at = current ? order.indexOf(current) : -1;
  if (at < 0) return dir > 0 ? order[0] : order[order.length - 1];
  return order[Math.min(order.length - 1, Math.max(0, at + dir))];
}

/** Roving focus in a list: the index a key moves to, or null for a key the
 *  list does not handle. */
export function rovingIndex(key: string, index: number, count: number): number | null {
  if (count <= 0) return null;
  switch (key) {
    case 'ArrowDown':
      return Math.min(count - 1, index + 1);
    case 'ArrowUp':
      return Math.max(0, index - 1);
    case 'Home':
      return 0;
    case 'End':
      return count - 1;
    default:
      return null;
  }
}

/** The thread a phone opens on: the most recent one that needs you (in
 *  `waitingNewestFirst`, kept only while it is still a live thread), else
 *  the most recently active one. */
export function initialThread(liveNewestFirst: readonly string[], waitingNewestFirst: readonly string[]): string | null {
  const live = new Set(liveNewestFirst);
  return waitingNewestFirst.find((id) => live.has(id)) ?? liveNewestFirst[0] ?? null;
}

/** Which Threads chord `e` is, if any (pure: unit-tested). */
export function threadChord(e: Pick<KeyboardEvent, 'key' | 'code' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'>): 'next' | 'previous' | 'needsYou' | null {
  if (!e.altKey || !(e.metaKey || e.ctrlKey) || e.shiftKey) return null;
  if (e.key === 'ArrowDown') return 'next';
  if (e.key === 'ArrowUp') return 'previous';
  // ⌥ turns N into a dead key on macOS: match the physical key too.
  if (e.code === 'KeyN' || e.key.toLowerCase() === 'n') return 'needsYou';
  return null;
}
