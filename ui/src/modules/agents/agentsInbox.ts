// The Agents inbox (patterns.md §10, the Threads concept): PTY sessions and
// structured threads in ONE list — needs you first, then Failed, then
// finished unread (Done), then the order the sort mode hands over (recent,
// manual or tokens); working sessions leave the list for the pinned
// "Working" fold. Pure (unit/agentsInbox.test.ts): the page passes in each
// row's class (lib/statusWords.ts `inboxClass`).
import type { Session } from '../../lib/api/types';
import type { InboxClass } from '../../lib/statusWords';

const RANK: Record<InboxClass, number> = { needs: 0, failed: 1, done: 2, working: 3, rest: 3 };

/** The main list and the Working fold. `list` comes in the sort mode's
 *  order ('recent' = newest activity first); urgency moves a row up, the
 *  incoming order breaks ties (stable). */
export function agentsInbox(
  list: readonly Session[],
  classOf: (s: Session) => InboxClass,
): { main: Session[]; working: Session[] } {
  const main: { s: Session; r: number; i: number }[] = [];
  const working: Session[] = [];
  list.forEach((s, i) => {
    const c = classOf(s);
    if (c === 'working') working.push(s);
    else main.push({ s, r: RANK[c], i });
  });
  main.sort((a, b) => a.r - b.r || a.i - b.i);
  return { main: main.map((x) => x.s), working };
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
