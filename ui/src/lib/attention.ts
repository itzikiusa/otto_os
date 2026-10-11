// The "needs you" flag as a raise/lower flag (the bell beside a session in
// the sidebar, Home's "Needs you", the dock badge):
//   • RAISED when the agent just finished a turn, or a prompt / permission
//     request is waiting on the operator — both arrive as the daemon's
//     `:waiting` notice (monitor.rs debounces the turn end);
//   • LOWERED as soon as the operator enters / focuses that session — even if
//     they do nothing there — or the agent resumes work, exits or is removed;
//   • it stays lowered until the NEXT turn end or prompt.
// Two timestamps per session decide it: the latest raise vs the latest time
// the operator saw (or the condition was resolved for) the session. Both are
// persisted (lib/stores/attention.svelte.ts), so a reload neither drops an
// unseen flag nor re-raises a seen one from an old notice. Pure: unit tested
// in unit/attention.test.ts.

export interface Attention {
  /** session id → when it was last raised (ms). */
  raised: Record<string, number>;
  /** session id → when the operator last saw it / it was resolved (ms). */
  seen: Record<string, number>;
}

export const emptyAttention = (): Attention => ({ raised: {}, seen: {} });

/** Raise `id` at `at`. While the operator is watching that very session the
 *  raise is seen at once (nothing to flag). Never moves a raise backwards. */
export function raise(a: Attention, id: string, at: number, watching = false): Attention {
  const raised = { ...a.raised, [id]: Math.max(at, a.raised[id] ?? -Infinity) };
  return watching ? { raised, seen: { ...a.seen, [id]: Math.max(at, a.seen[id] ?? -Infinity) } } : { ...a, raised };
}

/** The operator saw `id` at `at` (entered / focused it, sent it input), or
 *  its condition was resolved then (resumed work, exited): lowered until a
 *  later raise. */
export function see(a: Attention, id: string, at: number): Attention {
  if ((a.seen[id] ?? -Infinity) >= at) return a;
  return { ...a, seen: { ...a.seen, [id]: at } };
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
  if (!(id in a.raised) && !(id in a.seen)) return a;
  const raised = { ...a.raised };
  const seen = { ...a.seen };
  delete raised[id];
  delete seen[id];
  return { raised, seen };
}

/** Combine two windows' views (both write the same storage): the latest
 *  raise and the latest seen per session win. Times only move forward. */
export function mergeAttention(a: Attention, b: Attention): Attention {
  const max = (x: Record<string, number>, y: Record<string, number>) => {
    const out = { ...x };
    for (const [id, t] of Object.entries(y)) out[id] = Math.max(t, out[id] ?? -Infinity);
    return out;
  };
  return { raised: max(a.raised, b.raised), seen: max(a.seen, b.seen) };
}

/** Keep storage bounded: the `max` most recently touched sessions. */
export const ATTENTION_MAX = 400;

export function serializeAttention(a: Attention, max = ATTENTION_MAX): string {
  const ids = [...new Set([...Object.keys(a.raised), ...Object.keys(a.seen)])];
  const touched = (id: string) => Math.max(a.raised[id] ?? -Infinity, a.seen[id] ?? -Infinity);
  const keep = new Set(ids.sort((x, y) => touched(y) - touched(x)).slice(0, max));
  const pick = (m: Record<string, number>) => Object.fromEntries(Object.entries(m).filter(([id]) => keep.has(id)));
  return JSON.stringify({ raised: pick(a.raised), seen: pick(a.seen) });
}

/** Storage is a convenience, never trusted: anything malformed is dropped. */
export function parseAttention(raw: string | null): Attention {
  try {
    const v = JSON.parse(raw ?? '{}') as { raised?: unknown; seen?: unknown };
    const nums = (m: unknown): Record<string, number> => {
      if (!m || typeof m !== 'object' || Array.isArray(m)) return {};
      return Object.fromEntries(
        Object.entries(m as Record<string, unknown>).filter((e): e is [string, number] => typeof e[1] === 'number' && Number.isFinite(e[1])),
      );
    };
    return { raised: nums(v?.raised), seen: nums(v?.seen) };
  } catch {
    return emptyAttention();
  }
}
