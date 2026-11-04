// Structured runtime sessions (Threads) are not in the workspace store's
// session list; their "needs you" flags ride its attention store like any
// session's, and the badge counts add the flagged ones among the runtime ids
// kept here. The ids waiting on a person persist, so the count is right after
// a reload before Threads has loaded anything.
import type { Id } from '../api/types';
import { lsGet, lsSet } from '../storage';

const KEY = 'otto_runtime_waiting';

function read(): Record<Id, true> {
  try {
    const v: unknown = JSON.parse(lsGet(KEY) ?? '[]');
    return Object.fromEntries((Array.isArray(v) ? v : []).filter((x) => typeof x === 'string').map((x) => [x, true]));
  } catch {
    return {};
  }
}

const ids = $state<Record<Id, true>>(read());

/** A runtime session's provider started (or stopped) waiting on a person. */
export function noteRuntimeWaiting(id: Id, waiting: boolean): void {
  if (waiting === (ids[id] === true)) return;
  if (waiting) ids[id] = true;
  else delete ids[id];
  lsSet(KEY, JSON.stringify(Object.keys(ids)));
}

export function runtimeFlagged(needsYou: Record<Id, boolean>): number {
  return Object.keys(ids).filter((id) => needsYou[id]).length;
}
