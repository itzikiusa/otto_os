// Structured runtime sessions (Threads) are not in the workspace store's
// session list. The ids of those waiting on a person (an approval, a
// question, a secret) are kept here, from the rows' own status (on every
// list load) and the live `provider_session.updated` events, so the badge
// counts, Home's "waiting on you" rows and Threads' "Next that needs you"
// are right after a reload before Threads has loaded anything. Each id keeps
// the thread's title (Home names it) and when it started waiting (the most
// recent one opens first). Persisted.
import type { Id } from '../api/types';
import { lsGet, lsSet } from '../storage';

const KEY = 'otto_runtime_waiting';

export interface RuntimeWaiting {
  id: Id;
  /** The thread's title when it was last seen ('' when only the event named it). */
  title: string;
  /** Epoch ms it started waiting (this client's clock). */
  at: number;
}

/** Older builds stored a bare id array; both shapes read. */
export function parseWaiting(raw: string | null): Record<Id, RuntimeWaiting> {
  try {
    const v: unknown = JSON.parse(raw ?? '[]');
    const out: Record<Id, RuntimeWaiting> = {};
    for (const x of Array.isArray(v) ? v : []) {
      if (typeof x === 'string') out[x] = { id: x, title: '', at: 0 };
      else if (x && typeof x === 'object' && typeof (x as RuntimeWaiting).id === 'string') {
        const w = x as RuntimeWaiting;
        out[w.id] = { id: w.id, title: typeof w.title === 'string' ? w.title : '', at: Number.isFinite(w.at) ? w.at : 0 };
      }
    }
    return out;
  } catch {
    return {};
  }
}

const ids = $state<Record<Id, RuntimeWaiting>>(parseWaiting(lsGet(KEY)));

function persist(): void {
  lsSet(KEY, JSON.stringify(Object.values(ids)));
}

/** A runtime session started (or stopped) waiting on a person. `title`
 *  refreshes the remembered title without changing anything else. */
export function noteRuntimeWaiting(id: Id, waiting: boolean, title?: string): void {
  const had = ids[id];
  if (!waiting) {
    if (!had) return;
    delete ids[id];
    persist();
    return;
  }
  if (had && (title === undefined || title === had.title)) return;
  ids[id] = { id, title: title ?? had?.title ?? '', at: had?.at || Date.now() };
  persist();
}

/** The complete list of the caller's waiting threads (`GET /runtime-sessions`):
 *  every other remembered id is stale (answered or deleted elsewhere). */
export function syncRuntimeWaiting(rows: readonly { id: Id; title: string }[]): void {
  const keep = new Set(rows.map((r) => r.id));
  let changed = false;
  for (const id of Object.keys(ids)) {
    if (!keep.has(id)) {
      delete ids[id];
      changed = true;
    }
  }
  for (const r of rows) {
    const had = ids[r.id];
    if (had && had.title === r.title) continue;
    ids[r.id] = { id: r.id, title: r.title, at: had?.at || Date.now() };
    changed = true;
  }
  if (changed) persist();
}

/** The waiting threads, most recent first. */
export function runtimeWaiting(): RuntimeWaiting[] {
  return Object.values(ids).sort((a, b) => b.at - a.at);
}

/** Is `id` waiting on a person (from its row status or the live event)? */
export function isRuntimeWaiting(id: Id | null | undefined): boolean {
  return !!id && ids[id] !== undefined;
}

/** How many runtime threads wait on a person. A pending request needs the
 *  person until it is answered — looking at the thread does not lower it —
 *  so the count follows the rows, not the sticky "seen" flag. The flag map
 *  is kept as the argument for the workspace store's call. */
export function runtimeFlagged(_needsYou?: Record<Id, boolean>): number {
  return Object.keys(ids).length;
}
