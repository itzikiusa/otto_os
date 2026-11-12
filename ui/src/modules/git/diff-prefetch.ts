// Neighbour prefetch for the lazily loaded PR diff (DiffViewer + diff-load).
// A summary diff fetches a file's hunks only once it is expanded and near the
// viewport, so stepping to the next file always waited on a round trip. This
// warms the files AROUND the one being read: the next PREFETCH_AHEAD and the
// previous PREFETCH_BEHIND in the order the diff shows them, nearest first.
//
// It is the lowest priority there is:
//  • the browser has NET_SLOTS connections to the daemon; visible loads take
//    what they need first and prefetches only get the rest (at most
//    PREFETCH_MAX), and a visible load starting mid-prefetch aborts the
//    newest prefetches that no longer fit (they go back to the queue front);
//  • a new focus REPLACES the plan: queued work for files no longer near the
//    reader is dropped and in-flight work for them is aborted, and focus
//    changes are debounced, so a fast scroll past 200 files starts nothing;
//  • it stops while the client diff cache is near its byte cap.
// One plan is at most PREFETCH_AHEAD + PREFETCH_BEHIND + 1 files, so without
// the reader moving the total stays bounded (the huge-PR perf gate counts
// per-file fetches). A search plans more (up to SEARCH_CAP files) because the
// reader asked for content search over the whole PR.
//
// No Svelte, no DOM, no fetch: the host supplies the hooks.

export const PREFETCH_AHEAD = 10;
export const PREFETCH_BEHIND = 5;
/** Concurrent connections to one host (HTTP/1.1): visible loads + prefetches. */
export const NET_SLOTS = 6;
/** Prefetches in flight at most, even with every slot free. */
export const PREFETCH_MAX = 4;
/** Files a content search loads before asking to load the rest. */
export const SEARCH_CAP = 150;
/** Focus changes closer together than this are one (fast scrolling). */
export const FOCUS_DEBOUNCE_MS = 150;

export interface PrefetchHooks {
  /** Fetch one file's hunks and hand them to the view. Settles either way;
   *  must stop on `signal`. */
  run: (path: string, signal: AbortSignal) => Promise<unknown>;
  /** Still worth fetching: not loaded, not loading, not binary / too large /
   *  generated. Asked again right before each start. */
  wanted: (path: string) => boolean;
  /** Visible (high-priority) loads in flight right now. */
  busy: () => number;
  /** The client diff cache is near its byte cap. */
  full: () => boolean;
}

/** The neighbours of `order[at]` to warm, nearest first: the file itself,
 *  then two ahead for every one behind (reading goes down). */
export function neighbours(order: readonly string[], at: number, ahead = PREFETCH_AHEAD, behind = PREFETCH_BEHIND): string[] {
  if (order.length === 0) return [];
  const i = Math.max(0, Math.min(order.length - 1, at));
  const out = [order[i]];
  let a = 1;
  let b = 1;
  while (a <= ahead || b <= behind) {
    for (let k = 0; k < 2 && a <= ahead; k++, a++) if (i + a < order.length) out.push(order[i + a]);
    if (b <= behind) {
      if (i - b >= 0) out.push(order[i - b]);
      b++;
    }
  }
  return out;
}

/** A content search's plan: the files still without hunks (`wanted`) from
 *  the reader's position on, wrapping, capped at `cap`. `total` counts them
 *  all, so the host can say "searched N of M". */
export function searchPlan(order: readonly string[], at: number, wanted: (p: string) => boolean, cap = SEARCH_CAP): { paths: string[]; total: number } {
  const paths: string[] = [];
  let total = 0;
  const n = order.length;
  const start = n === 0 ? 0 : Math.max(0, Math.min(n - 1, at));
  for (let k = 0; k < n; k++) {
    const p = order[(start + k) % n];
    if (!wanted(p)) continue;
    total++;
    if (paths.length < cap) paths.push(p);
  }
  return { paths, total };
}

export class PrefetchScheduler {
  #hooks: PrefetchHooks;
  #queue: string[] = [];
  /** In flight, in start order (the newest yield first). */
  #active = new Map<string, AbortController>();
  #timer: ReturnType<typeof setTimeout> | null = null;

  constructor(hooks: PrefetchHooks) {
    this.#hooks = hooks;
  }

  /** Files queued or in flight. */
  get pending(): number {
    return this.#queue.length + this.#active.size;
  }

  /** Is `path` being prefetched right now? */
  active(path: string): boolean {
    return this.#active.has(path);
  }

  /** Replace the plan with `paths` (in priority order). In-flight work that
   *  is not in it is aborted; what is already in flight and still planned
   *  keeps going. */
  plan(paths: readonly string[]): void {
    this.#cancelTimer();
    const want = new Set(paths);
    for (const [p, ctl] of this.#active) {
      if (!want.has(p)) {
        ctl.abort();
        this.#active.delete(p);
      }
    }
    this.#queue = paths.filter((p) => !this.#active.has(p));
    this.pump();
  }

  /** `plan(make())` once the focus has been still for `ms` (a newer call
   *  replaces a pending one). `make` runs at fire time, on fresh state. */
  planSoon(make: () => readonly string[], ms = FOCUS_DEBOUNCE_MS): void {
    this.#cancelTimer();
    this.#timer = setTimeout(() => {
      this.#timer = null;
      this.plan(make());
    }, ms);
  }

  /** How many prefetches may be in flight right now. */
  limit(): number {
    return Math.max(0, Math.min(PREFETCH_MAX, NET_SLOTS - this.#hooks.busy()));
  }

  /** A visible load started: abort the newest prefetches that no longer fit
   *  in the free slots and put them back at the front of the queue. */
  yield(): void {
    const over = this.#active.size - this.limit();
    if (over <= 0) return;
    const newest = [...this.#active.keys()].slice(-over);
    for (const p of newest) {
      this.#active.get(p)?.abort();
      this.#active.delete(p);
    }
    this.#queue = [...newest, ...this.#queue];
  }

  /** Forget `path` (a visible load took it over): unqueue it and abort its
   *  prefetch, if any. */
  drop(path: string): void {
    const ctl = this.#active.get(path);
    if (ctl) {
      ctl.abort();
      this.#active.delete(path);
    }
    const i = this.#queue.indexOf(path);
    if (i >= 0) this.#queue.splice(i, 1);
  }

  /** Start queued work while there are slots, it is wanted and the cache has
   *  room. Called after every plan and whenever a load (any) settles. */
  pump(): void {
    while (this.#queue.length > 0 && this.#active.size < this.limit()) {
      if (this.#hooks.full()) {
        // Near the byte cap: stop warming. What is queued is dropped, not
        // parked — the next focus change plans again from fresh state.
        this.#queue = [];
        return;
      }
      const p = this.#queue.shift() as string;
      if (this.#active.has(p) || !this.#hooks.wanted(p)) continue;
      const ctl = new AbortController();
      this.#active.set(p, ctl);
      const settle = (): void => {
        if (this.#active.get(p) === ctl) this.#active.delete(p);
        if (!ctl.signal.aborted) this.pump();
      };
      this.#hooks.run(p, ctl.signal).then(settle, settle);
    }
  }

  /** Abort everything and forget the plan (a new diff, unmount). */
  reset(): void {
    this.#cancelTimer();
    for (const ctl of this.#active.values()) ctl.abort();
    this.#active.clear();
    this.#queue = [];
  }

  #cancelTimer(): void {
    if (this.#timer !== null) clearTimeout(this.#timer);
    this.#timer = null;
  }
}
