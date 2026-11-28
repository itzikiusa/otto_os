// The Threads sidebar's list as one sequence: the order "next / previous
// thread" and "Next that needs you" walk (pinned first, then every project's
// inbox order, then the folded origin groups — folded rows included, so a
// thread behind a fold is still reached). ↑/↓ / Home / End between the rows
// are the shared list walk (ListPane's use:listNav).
import type { Session } from '../../lib/api/types';

/** Unique live thread ids in sidebar order. */
export function sidebarOrder(
  pinned: readonly Session[],
  groups: readonly { items: readonly { session: Session }[] }[],
  side: readonly { items: readonly Session[] }[],
): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  const add = (s: Session): void => {
    if (s.archived || seen.has(s.id)) return;
    seen.add(s.id);
    out.push(s.id);
  };
  pinned.forEach(add);
  for (const g of groups) for (const r of g.items) add(r.session);
  for (const g of side) g.items.forEach(add);
  return out;
}
