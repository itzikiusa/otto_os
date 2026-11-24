// The Agents inbox (patterns.md §10, the Threads concept): PTY sessions and
// structured threads in ONE list — needs you first, then Failed, then
// finished unread (Done), then the working ones, then the rest in the order
// the sort mode hands over (recent, manual or tokens) — all in the one list:
// with six agents running the live sessions ARE the list, never a cramped
// fold under an empty one. Pure (unit/agentsInbox.test.ts): the page passes
// in each row's class (lib/statusWords.ts `inboxClass`).
import type { Session } from '../../lib/api/types';
import type { InboxClass } from '../../lib/statusWords';

const RANK: Record<InboxClass, number> = { needs: 0, failed: 1, done: 2, working: 3, rest: 4 };

/** The inbox list. `list` comes in the sort mode's order ('recent' = newest
 *  activity first); urgency moves a row up, the incoming order breaks ties (stable). */
export function agentsInbox(list: readonly Session[], classOf: (s: Session) => InboxClass): Session[] {
  return list
    .map((s, i) => ({ s, r: RANK[classOf(s)], i }))
    .sort((a, b) => a.r - b.r || a.i - b.i)
    .map((x) => x.s);
}

/** Newest activity first (the 'recent' sort; ISO strings compare as text). */
export function byRecent<T extends { last_active_at: string }>(list: readonly T[]): T[] {
  return [...list].sort((a, b) => b.last_active_at.localeCompare(a.last_active_at));
}

/** Search: title, provider, folder, branch or workspace name. */
export function matchesQuery(s: Session, q: string, extra: readonly (string | null | undefined)[] = []): boolean {
  const n = q.trim().toLowerCase();
  if (!n) return true;
  return [s.title, s.provider, s.cwd, ...extra].some((v) => (v ?? '').toLowerCase().includes(n));
}
