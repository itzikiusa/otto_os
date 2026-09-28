// Polling discipline for every UI poller (backlog X1). The webview talks to the
// daemon over HTTP/1.1 with WebKit's 6-connection cap per host, so a poll that
// stacks requests, keeps going while the window is hidden, or outlives its page
// starves every interactive fetch in the app. This is the one sanctioned shape:
//
// - a setTimeout CHAIN, not setInterval: the next tick is scheduled only after
//   the previous run settles, so a slow daemon never gets overlapping requests
//   (the in-flight guard);
// - ticks pause while the document is hidden (or run at a slower `hiddenMs`
//   cadence when the poll must keep a native badge fresh) and a tick skipped
//   while hidden runs the moment the document is visible again;
// - consecutive failures back the cadence off (×2 per failure, capped);
// - ±jitter so many pollers started together don't fire in lockstep;
// - `stop()` aborts the in-flight run's `AbortSignal`, so unmount releases the
//   socket instead of letting a dead page hold one of the six connections.
//
// Exactly ONE chain exists per poller: `now()` during an in-flight run does not
// start a second tick (both would reschedule on settle) — it queues one rerun
// that replaces the scheduled tick when the current run settles.

export interface Poller {
  stop(): void;
  /** Run immediately (manual refresh / live event) and restart the cadence. */
  now(): void;
  /** Re-arm the pending tick at the CURRENT cadence without running (the
   *  cadence inputs changed — e.g. liveQuery's event socket came up). */
  reschedule?(): void;
}

export interface PollOptions {
  /** Base cadence between the END of one run and the start of the next. */
  ms: number;
  /** Lower bound on the cadence (default 1000 ms). */
  floorMs?: number;
  /** Maximum backoff multiplier after consecutive failures (default 8). */
  maxBackoff?: number;
  /** While the document is hidden: `'pause'` (default) skips ticks and runs one
   *  on return; a number keeps polling at that (slower) cadence instead — only
   *  for pollers whose result is visible outside the webview (a menu-bar
   *  badge). */
  hidden?: 'pause' | number;
  /** ± fraction of the delay to randomise (default 0.1; 0 = exact). */
  jitter?: number;
  /** Run once right away (default true). `false` waits one cadence first. */
  immediate?: boolean;
  /** Stopping this signal stops the poller (an owner's unmount signal). */
  signal?: AbortSignal;
}

/** A run reports failure by resolving `false` or throwing; anything else
 *  (`true`, `undefined`) is success. The signal aborts when the poller stops. */
export type PollRun = (signal: AbortSignal) => Promise<boolean | void> | boolean | void;

function docHidden(): boolean {
  return typeof document !== 'undefined' && document.visibilityState === 'hidden';
}

export function pollWhileVisible(run: PollRun, opts: PollOptions): Poller {
  const floor = opts.floorMs ?? 1000;
  const maxBackoff = Math.max(1, opts.maxBackoff ?? 8);
  const jitter = Math.max(0, Math.min(0.5, opts.jitter ?? 0.1));
  const hiddenMode = opts.hidden ?? 'pause';
  let stopped = false;
  let failures = 0;
  let handle: ReturnType<typeof setTimeout> | undefined;
  let inFlight: AbortController | null = null;
  let rerun = false;
  /** A tick came due while hidden (pause mode) — run it on visibility. */
  let owed = false;

  const delay = (): number => {
    // `ms` / a numeric `hidden` are re-read per schedule: liveQuery (lib/live.ts)
    // passes getters so the cadence follows the event socket's state.
    const base = Math.max(opts.ms, floor);
    const hiddenNow = opts.hidden ?? 'pause';
    const hiddenBase = typeof hiddenNow === 'number' && docHidden() ? Math.max(hiddenNow, floor) : base;
    const backoff = Math.min(2 ** failures, maxBackoff);
    const d = hiddenBase * backoff;
    return jitter > 0 ? Math.round(d * (1 - jitter + Math.random() * 2 * jitter)) : d;
  };

  const schedule = (): void => {
    if (stopped) return;
    if (handle !== undefined) clearTimeout(handle);
    handle = setTimeout(() => void tick(), delay());
  };

  const tick = async (): Promise<void> => {
    if (stopped) return;
    if (inFlight) {
      rerun = true;
      return;
    }
    if (hiddenMode === 'pause' && docHidden()) {
      // No timer while hidden: the visibilitychange listener pays the debt.
      owed = true;
      return;
    }
    owed = false;
    const ctl = new AbortController();
    inFlight = ctl;
    try {
      const ok = await run(ctl.signal);
      failures = ok === false ? failures + 1 : 0;
    } catch {
      failures += 1;
    } finally {
      inFlight = null;
    }
    if (stopped) return;
    if (rerun) {
      rerun = false;
      void tick();
      return;
    }
    schedule();
  };

  const onVisibility = (): void => {
    if (stopped || docHidden()) return;
    if (owed) {
      if (handle !== undefined) clearTimeout(handle);
      void tick();
    } else if (typeof hiddenMode === 'number') {
      // Back from a slow hidden cadence: re-arm at the visible one.
      if (!inFlight) schedule();
    }
  };

  const stop = (): void => {
    if (stopped) return;
    stopped = true;
    if (handle !== undefined) clearTimeout(handle);
    inFlight?.abort();
    inFlight = null;
    if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', onVisibility);
    opts.signal?.removeEventListener('abort', stop);
  };

  if (opts.signal?.aborted) {
    stopped = true;
    return { stop() {}, now() {} };
  }
  opts.signal?.addEventListener('abort', stop);
  if (typeof document !== 'undefined') document.addEventListener('visibilitychange', onVisibility);
  if (opts.immediate === false) schedule();
  else void tick();

  return {
    stop,
    now() {
      if (stopped) return;
      if (handle !== undefined) clearTimeout(handle);
      failures = 0;
      void tick();
    },
    reschedule() {
      // In flight → it reschedules on settle; owed → the visibility return runs it.
      if (stopped || inFlight || owed) return;
      schedule();
    },
  };
}

/** Run `task` over `items` with at most `limit` in flight (order of results
 *  matches `items`). For fan-outs that must not take every webview socket. */
export async function mapLimit<T, R>(items: readonly T[], limit: number, task: (item: T, index: number) => Promise<R>): Promise<R[]> {
  const out = new Array<R>(items.length);
  let next = 0;
  const worker = async (): Promise<void> => {
    while (next < items.length) {
      const i = next++;
      out[i] = await task(items[i], i);
    }
  };
  await Promise.all(Array.from({ length: Math.max(1, Math.min(limit, items.length)) }, worker));
  return out;
}
