// Home boxes' auto-refresh. The discipline (setTimeout chain with an in-flight
// guard, hidden pause, failure backoff, single chain per poller) now lives in
// the shared `lib/poll.ts`; this keeps the boxes' original `poll(run, ms)`
// shape and their 5 s floor.
import { pollWhileVisible, type Poller } from '../../../lib/poll';
import { liveQuery, type LiveEvent } from '../../../lib/live';

export type { Poller };

/** `run` receives the poller's AbortSignal (aborted on `stop()` — a box that
 *  passes it on releases its socket when it unmounts or goes off screen). */
export function poll(run: (signal?: AbortSignal) => Promise<boolean>, ms: number, immediate = true): Poller {
  return pollWhileVisible((signal) => run(signal), { ms, floorMs: 5000, jitter: 0, immediate });
}

/**
 * Remembers when a box's `run` last succeeded, and for which inputs (`key`:
 * the window, workspace, limit…). Every Home space stays mounted (review 06
 * F3) and only the active one polls; when a space comes back, `start` says
 * whether its data is stale enough to fetch at once or can wait for the next
 * tick — a slide back no longer re-fetches every box. A changed key always
 * fetches at once.
 */
export function freshness(run: (signal?: AbortSignal) => Promise<boolean>): {
  run: (signal?: AbortSignal) => Promise<boolean>;
  /** Call when (re)starting the poller; returns `immediate` for it. */
  start: (key: string, ms: number) => boolean;
} {
  let at = 0;
  let atKey = '';
  let curKey = '';
  return {
    run: async (signal?: AbortSignal) => {
      const key = curKey;
      const ok = await run(signal);
      if (ok) {
        at = Date.now();
        atKey = key;
      }
      return ok;
    },
    start: (key, ms) => {
      curKey = key;
      return !(at > 0 && atKey === key && Date.now() - at < ms);
    },
  };
}

/** Event-fed variant (TRANSPORT_PLAN stage 2): re-run when one of `on`
 *  arrives (bursts coalesced), a 5-min safety net while the event socket is
 *  up, and the original `ms` cadence only while it is down. */
export function livePoll(
  run: (signal?: AbortSignal) => Promise<boolean>,
  ms: number,
  on: readonly string[],
  opts: { match?: (ev: LiveEvent) => boolean; debounceMs?: number; maxWaitMs?: number; minIntervalMs?: number; immediate?: boolean } = {},
): Poller {
  // Hand the poller's signal through: `stop()` (box unmounted / off screen)
  // must abort an in-flight fetch, as `poll` above does.
  return liveQuery({ run: (signal) => run(signal), on, fallbackMs: ms, floorMs: 5000, jitter: 0, ...opts });
}
