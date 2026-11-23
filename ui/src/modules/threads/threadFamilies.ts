// The Threads sidebar's inbox order inside a project: needs-you first, then
// working, then failed, then most recent (a family by its most urgent member) — and threads that belong to a thread (a linked
// child, a delegated worker, a council's seats) nested under it, foldable.
// A child whose parent is not in the list (closed, archived, filtered out)
// still shows at the top level, marked detached. Pure
// (unit/threadsChildren.test.ts).
import type { Session } from '../../lib/api/types';

export interface FamilyRow {
  session: Session;
  /** Nested under its parent. */
  child: boolean;
  /** The ids of the threads nested under this one (all of them, folded or not). */
  kids: string[];
  /** Its parent is not in the list. */
  detached: boolean;
}

/** `parentOf` is threadsHelpers' `delegatedParentId` (passed in: this file stays free of stores). */
export function inboxOrder(
  list: readonly Session[],
  score: (s: Session) => number,
  parentOf: (s: Session) => string | null,
  folded: (id: string) => boolean = () => false,
  /** Recency (ISO time, newest first): last activity unless the caller knows better. */
  at: (s: Session) => string = (s) => s.last_active_at,
): FamilyRow[] {
  const sorted = [...list].sort((a, b) => score(a) - score(b) || at(b).localeCompare(at(a)));
  const present = new Set(sorted.map((s) => s.id));
  const childrenOf = new Map<string, Session[]>();
  const roots: { s: Session; detached: boolean }[] = [];
  for (const s of sorted) {
    const parentId = parentOf(s);
    if (parentId && parentId !== s.id && present.has(parentId)) {
      childrenOf.set(parentId, [...(childrenOf.get(parentId) ?? []), s]);
    } else roots.push({ s, detached: !!parentId && parentId !== s.id });
  }
  // A family sorts by its most urgent member: a parent whose child waits on
  // you rises with it (ties keep the order above — urgency, then recency).
  const scoreOf = new Map(sorted.map((s) => [s.id, score(s)] as const));
  const famScore = (id: string, seen = new Set<string>()): number => {
    if (seen.has(id)) return Number.POSITIVE_INFINITY;
    seen.add(id);
    let best = scoreOf.get(id) ?? Number.POSITIVE_INFINITY;
    for (const kid of childrenOf.get(id) ?? []) best = Math.min(best, famScore(kid.id, seen));
    return best;
  };
  const rank = new Map(roots.map((r, i) => [r.s.id, i] as const));
  const fam = new Map(roots.map((r) => [r.s.id, famScore(r.s.id)] as const));
  roots.sort((a, b) => fam.get(a.s.id)! - fam.get(b.s.id)! || rank.get(a.s.id)! - rank.get(b.s.id)!);
  const out: FamilyRow[] = [];
  const visit = (s: Session, child: boolean, detached: boolean): void => {
    const kids = childrenOf.get(s.id) ?? [];
    out.push({ session: s, child, kids: kids.map((k) => k.id), detached });
    if (folded(s.id)) return;
    for (const kid of kids) visit(kid, true, false);
  };
  for (const r of roots) visit(r.s, false, r.detached);
  return out;
}
