// The Threads sidebar's inbox order inside a project: needs-you first, then
// working, then most recent — and threads that belong to a thread (a linked
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
): FamilyRow[] {
  const sorted = [...list].sort((a, b) => score(a) - score(b) || b.last_active_at.localeCompare(a.last_active_at));
  const present = new Set(sorted.map((s) => s.id));
  const childrenOf = new Map<string, Session[]>();
  const roots: { s: Session; detached: boolean }[] = [];
  for (const s of sorted) {
    const parentId = parentOf(s);
    if (parentId && parentId !== s.id && present.has(parentId)) {
      childrenOf.set(parentId, [...(childrenOf.get(parentId) ?? []), s]);
    } else roots.push({ s, detached: !!parentId && parentId !== s.id });
  }
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
