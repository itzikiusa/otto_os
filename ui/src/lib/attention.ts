// The "needs you" flag as a raise/lower flag (the bell beside a session in
// the sidebar, Home's "Needs you", the dock badge):
//   • RAISED when the agent just finished a turn, or a prompt / permission
//     request is waiting on the operator — the daemon's `session_attention`
//     event (sent whatever the notification settings say; monitor.rs
//     debounces the turn end) or its `:waiting` notice;
//   • LOWERED as soon as the operator enters / focuses that session — even if
//     they do nothing there — or the agent resumes work, exits or is removed;
//   • it stays lowered until the NEXT turn end or prompt.
//
// Clocks never mix. `raised` / `seen` are THIS client's clock (a raise is
// stamped when it is received, a lower when it happens), so a skewed remote
// daemon can't hide or pin a flag. `notice` is the DAEMON's clock: the newest
// notice time the operator had already answered for a session, compared only
// with other notice times when a reload re-derives flags from unread notices.
// All three are persisted (workspace store), so a reload neither drops an
// unseen flag nor re-raises a seen one. Pure: unit/attention.test.ts.

export interface Attention {
  /** session id → when it was last raised (client ms). */
  raised: Record<string, number>;
  /** session id → when the operator last saw it / it was resolved (client ms). */
  seen: Record<string, number>;
  /** session id → newest answered notice time (daemon ms). */
  notice: Record<string, number>;
}

export const emptyAttention = (): Attention => ({ raised: {}, seen: {}, notice: {} });

const later = (m: Record<string, number>, id: string, at: number): Record<string, number> =>
  (m[id] ?? -Infinity) >= at ? m : { ...m, [id]: at };

/** Raise `id` at `at` (client clock). While the operator is watching that
 *  very session the raise is seen at once. Never moves a raise backwards.
 *  `noticeAt` (daemon clock): the notice behind the raise, answered at once
 *  when watching. */
export function raise(a: Attention, id: string, at: number, watching = false, noticeAt?: number): Attention {
  const next = { ...a, raised: later(a.raised, id, at) };
  if (!watching) return next;
  return { ...next, seen: later(a.seen, id, at), notice: noticeAt === undefined ? a.notice : later(a.notice, id, noticeAt) };
}

/** The operator saw `id` at `at` (entered / focused it, sent it input), or
 *  its condition was resolved then (resumed work, exited): lowered until a
 *  later raise. `noticeAt` (daemon clock): the newest notice it answers. */
export function see(a: Attention, id: string, at: number, noticeAt?: number): Attention {
  const seen = later(a.seen, id, at);
  const notice = noticeAt === undefined ? a.notice : later(a.notice, id, noticeAt);
  return seen === a.seen && notice === a.notice ? a : { ...a, seen, notice };
}

/** A reload re-derives a flag from an unread notice made at `noticeAt`
 *  (daemon clock): only a notice newer than the last one answered counts. */
export function noticeIsNew(a: Attention, id: string, noticeAt: number): boolean {
  return noticeAt > (a.notice[id] ?? -Infinity);
}

/** Is `id`'s flag up: raised after it was last seen? */
export function flagged(a: Attention, id: string): boolean {
  const r = a.raised[id];
  return r !== undefined && r > (a.seen[id] ?? -Infinity);
}

export function flaggedIds(a: Attention): string[] {
  return Object.keys(a.raised).filter((id) => flagged(a, id));
}

/** Drop a session entirely (removed for good). */
export function forget(a: Attention, id: string): Attention {
  if (!(id in a.raised) && !(id in a.seen) && !(id in a.notice)) return a;
  const drop = (m: Record<string, number>) => {
    const out = { ...m };
    delete out[id];
    return out;
  };
  return { raised: drop(a.raised), seen: drop(a.seen), notice: drop(a.notice) };
}

/** Combine two windows' views (both write the same storage): the latest
 *  value of each map per session wins. Times only move forward. */
export function mergeAttention(a: Attention, b: Attention): Attention {
  const max = (x: Record<string, number>, y: Record<string, number>) => {
    const out = { ...x };
    for (const [id, t] of Object.entries(y)) out[id] = Math.max(t, out[id] ?? -Infinity);
    return out;
  };
  return { raised: max(a.raised, b.raised), seen: max(a.seen, b.seen), notice: max(a.notice, b.notice) };
}

/** Keep storage bounded: the `max` most recently touched sessions. */
export const ATTENTION_MAX = 400;

export function serializeAttention(a: Attention, max = ATTENTION_MAX): string {
  const ids = [...new Set([...Object.keys(a.raised), ...Object.keys(a.seen), ...Object.keys(a.notice)])];
  const touched = (id: string) => Math.max(a.raised[id] ?? -Infinity, a.seen[id] ?? -Infinity);
  const keep = new Set(ids.sort((x, y) => touched(y) - touched(x)).slice(0, max));
  const pick = (m: Record<string, number>) => Object.fromEntries(Object.entries(m).filter(([id]) => keep.has(id)));
  return JSON.stringify({ raised: pick(a.raised), seen: pick(a.seen), notice: pick(a.notice) });
}

/** Storage is a convenience, never trusted: anything malformed is dropped. */
export function parseAttention(raw: string | null): Attention {
  try {
    const v = JSON.parse(raw ?? '{}') as { raised?: unknown; seen?: unknown; notice?: unknown };
    const nums = (m: unknown): Record<string, number> => {
      if (!m || typeof m !== 'object' || Array.isArray(m)) return {};
      return Object.fromEntries(
        Object.entries(m as Record<string, unknown>).filter((e): e is [string, number] => typeof e[1] === 'number' && Number.isFinite(e[1])),
      );
    };
    return { raised: nums(v?.raised), seen: nums(v?.seen), notice: nums(v?.notice) };
  } catch {
    return emptyAttention();
  }
}
