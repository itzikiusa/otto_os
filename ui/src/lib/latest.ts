/**
 * Stale-response guards for async loads that write into UI state.
 *
 * The bug this prevents: two loads for the same slot overlap (open A then B,
 * a background poll racing a user action, a start+finish event pair firing two
 * refreshes) and the OLDER response lands last, overwriting the newer one.
 * Every such site needs a generation check between the await and the write;
 * these helpers make that one line instead of a hand-rolled `seq` per store.
 *
 *   const runs = latestOnly();
 *   async function loadRuns(id: string) {
 *     const t = runs.begin();               // supersedes any earlier load
 *     const rows = await api.get(...);
 *     if (!t.current) return;               // a newer load (or cancel) won
 *     this.runs = rows;
 *   }
 *
 * `begin()` also aborts the previous ticket's `signal`, so passing
 * `{ signal: t.signal }` to fetch-style calls cancels the superseded request
 * instead of only ignoring its result. `cancel()` (e.g. on unmount / dispose)
 * invalidates the in-flight ticket without starting a new one.
 *
 * Pure TS (no runes) so stores, components and node unit tests can all use it.
 */

/** One load's claim on the slot. */
export interface LatestTicket {
  /** Monotonic generation number for this ticket. */
  readonly gen: number;
  /** True while no later `begin()` / `cancel()` has happened. */
  readonly current: boolean;
  /** Aborted as soon as the ticket is superseded or cancelled. */
  readonly signal: AbortSignal;
}

export interface Latest {
  /** Start a new load: supersedes (and aborts) the previous ticket. */
  begin(): LatestTicket;
  /** Invalidate the in-flight ticket without starting a new one. */
  cancel(): void;
  /** Current generation (bumped by every begin/cancel). */
  readonly gen: number;
  /**
   * Convenience: run `fn` under a fresh ticket and resolve to
   * `{ ok: true, value }` when it is still the latest when it settles, or
   * `{ ok: false }` when superseded. Rejections from a superseded run are
   * swallowed (they belong to a load nobody is waiting for); rejections from
   * the current run propagate.
   */
  run<T>(fn: (signal: AbortSignal) => Promise<T>): Promise<{ ok: true; value: T } | { ok: false }>;
}

/** Create a generation guard for one logical slot (one list, one detail pane). */
export function latestOnly(): Latest {
  let gen = 0;
  let ctrl: AbortController | null = null;

  function supersede() {
    gen += 1;
    ctrl?.abort();
    ctrl = null;
  }

  function begin(): LatestTicket {
    supersede();
    const mine = gen;
    const c = new AbortController();
    ctrl = c;
    return {
      gen: mine,
      get current() {
        return gen === mine;
      },
      signal: c.signal,
    };
  }

  return {
    begin,
    cancel: supersede,
    get gen() {
      return gen;
    },
    async run<T>(fn: (signal: AbortSignal) => Promise<T>) {
      const t = begin();
      try {
        const value = await fn(t.signal);
        return t.current ? { ok: true as const, value } : { ok: false as const };
      } catch (e) {
        if (!t.current) return { ok: false as const };
        throw e;
      }
    },
  };
}

/**
 * Keyed variant: one independent generation per key (per run id, per repo id),
 * for stores that load several slots concurrently and only need each slot's
 * own responses ordered.
 */
export function latestByKey<K = string>(): {
  begin(key: K): LatestTicket;
  cancel(key: K): void;
  cancelAll(): void;
} {
  const slots = new Map<K, Latest>();
  const slot = (key: K) => {
    let s = slots.get(key);
    if (!s) {
      s = latestOnly();
      slots.set(key, s);
    }
    return s;
  };
  return {
    begin: (key) => slot(key).begin(),
    cancel: (key) => slots.get(key)?.cancel(),
    cancelAll: () => {
      for (const s of slots.values()) s.cancel();
    },
  };
}
