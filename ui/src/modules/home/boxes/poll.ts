// Home boxes' auto-refresh. The discipline (setTimeout chain with an in-flight
// guard, hidden pause, failure backoff, single chain per poller) now lives in
// the shared `lib/poll.ts`; this keeps the boxes' original `poll(run, ms)`
// shape and their 5 s floor.
import { pollWhileVisible, type Poller } from '../../../lib/poll';
import { liveQuery, type LiveEvent } from '../../../lib/live';

export type { Poller };

export function poll(run: () => Promise<boolean>, ms: number): Poller {
  return pollWhileVisible(() => run(), { ms, floorMs: 5000, jitter: 0 });
}

/** Event-fed variant (TRANSPORT_PLAN stage 2): re-run when one of `on`
 *  arrives (bursts coalesced), a 5-min safety net while the event socket is
 *  up, and the original `ms` cadence only while it is down. */
export function livePoll(
  run: () => Promise<boolean>,
  ms: number,
  on: readonly string[],
  opts: { match?: (ev: LiveEvent) => boolean; debounceMs?: number; maxWaitMs?: number; minIntervalMs?: number } = {},
): Poller {
  return liveQuery({ run: () => run(), on, fallbackMs: ms, floorMs: 5000, jitter: 0, ...opts });
}
