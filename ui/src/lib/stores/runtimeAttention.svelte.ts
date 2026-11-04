// Structured runtime sessions (Threads) are not in the workspace store's
// session list; their "needs you" flags ride its attention store like any
// session's, and the badge counts add the ones flagged among the ids seen here.
import type { Id } from '../api/types';

const ids = $state<Record<Id, true>>({});

export function noteRuntimeSession(id: Id): void {
  ids[id] = true;
}

export function runtimeFlagged(needsYou: Record<Id, boolean>): number {
  return Object.keys(ids).filter((id) => needsYou[id]).length;
}
