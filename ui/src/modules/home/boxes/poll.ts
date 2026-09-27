// Home boxes' auto-refresh. The discipline (setTimeout chain with an in-flight
// guard, hidden pause, failure backoff, single chain per poller) now lives in
// the shared `lib/poll.ts`; this keeps the boxes' original `poll(run, ms)`
// shape and their 5 s floor.
import { pollWhileVisible, type Poller } from '../../../lib/poll';

export type { Poller };

export function poll(run: () => Promise<boolean>, ms: number): Poller {
  return pollWhileVisible(() => run(), { ms, floorMs: 5000, jitter: 0 });
}
