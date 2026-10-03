// Cadence schedules for pollers whose every tick costs real work behind the
// daemon — an `aws` CLI process (a Python interpreter, ~0.3 s CPU each), a
// Kafka consume over a tunnel. Pure on purpose: the views own their timers (or
// hand a cadence to `pollWhileVisible` through a `get ms()` getter, which it
// re-reads per schedule), and the schedules themselves are unit-tested
// (unit/pollBackoff.test.ts) instead of being re-derived in every view.

/** Status polls of a server-side job (Athena query, Logs Insights query): a
 *  10-minute scan polled every second burned ~30% of a core in `aws`
 *  processes. Most queries finish in the first few seconds, so start tight and
 *  stretch: 1,1,2,2,3,5 s… capped at 5 s. */
export const STATUS_POLL_BACKOFF: readonly number[] = [1000, 1000, 2000, 2000, 3000, 5000];
/** While the window is hidden nobody is watching the spinner — 15 s. */
export const STATUS_POLL_HIDDEN_MS = 15_000;

/** Delay before status poll number `n` (0-based) of one job. */
export function statusPollMs(n: number, hidden: boolean): number {
  const ms = STATUS_POLL_BACKOFF[Math.max(0, Math.min(n, STATUS_POLL_BACKOFF.length - 1))];
  return hidden ? Math.max(ms, STATUS_POLL_HIDDEN_MS) : ms;
}

export interface AdaptiveBounds {
  /** Cadence while ticks keep returning data. */
  min: number;
  /** Ceiling the idle back-off stops at. */
  max: number;
}

/** Live-tail cadence: snap back to `min` the moment a tick returns something
 *  new; double on every empty tick up to `max`. A busy stream stays live, an
 *  idle one stops paying a full fetch every `min` ms. */
export function adaptiveDelay(prev: number, gotData: boolean, { min, max }: AdaptiveBounds): number {
  if (gotData) return min;
  return Math.min(max, Math.max(min, prev * 2));
}

export interface AdaptiveCadence {
  /** The delay before the next tick (pass as `get ms()` to pollWhileVisible). */
  readonly ms: number;
  /** Feed the outcome of the tick that just settled. */
  record(gotData: boolean): void;
  /** Back to `min` (a new tail started, the filter changed…). */
  reset(): void;
}

/** A stateful `adaptiveDelay` for one tail. */
export function adaptiveCadence(bounds: AdaptiveBounds): AdaptiveCadence {
  let ms = bounds.min;
  return {
    get ms() {
      return ms;
    },
    record(gotData) {
      ms = adaptiveDelay(ms, gotData, bounds);
    },
    reset() {
      ms = bounds.min;
    },
  };
}

export interface QuietBounds extends AdaptiveBounds {
  /** Consecutive unchanged samples tolerated before the cadence stretches. */
  quietAfter: number;
}

/** Dashboard-sample cadence (Kafka Overview metrics): stay at `min` while the
 *  samples change; after `quietAfter` unchanged samples in a row, back off
 *  toward `max` (doubling); the first changed sample snaps back to `min`. */
export function quietCadence({ min, max, quietAfter }: QuietBounds): AdaptiveCadence & {
  /** Feed whether the sample just received differs from the previous one. */
  sample(changed: boolean): void;
} {
  const inner = adaptiveCadence({ min, max });
  let unchanged = 0;
  return {
    get ms() {
      return inner.ms;
    },
    record(gotData) {
      inner.record(gotData);
    },
    sample(changed) {
      unchanged = changed ? 0 : unchanged + 1;
      inner.record(unchanged < quietAfter);
    },
    reset() {
      unchanged = 0;
      inner.reset();
    },
  };
}
