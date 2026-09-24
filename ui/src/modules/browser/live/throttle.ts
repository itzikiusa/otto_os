// Remote live browser — pointer-move coalescing. No imports (unit-tested).
//
// A trackpad fires pointermove at 120 Hz+; every one of them is a CDP round
// trip on the daemon. Moves are therefore COALESCED: at most one per
// `intervalMs`, and it always carries the LATEST position (a stale position
// sent late is worse than none). A trailing send guarantees the final resting
// position arrives, so hover states settle where the pointer actually stopped.
//
// Anything that isn't a move (down/up/wheel/key) must call `flush()` first so
// the remote sees the pointer at the click position before the click.

export interface Clock {
  now(): number;
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

export interface Coalescer<T> {
  /** Offer the latest value; sends now or schedules a trailing send. */
  push(value: T): void;
  /** Send any pending value immediately (before a click, on blur). */
  flush(): void;
  /** Drop any pending value without sending (on disconnect / unmount). */
  cancel(): void;
  readonly pending: boolean;
}

export function coalesce<T>(send: (value: T) => void, intervalMs: number, clock: Clock): Coalescer<T> {
  let last = -Infinity;
  let pending: { value: T } | null = null;
  let timer: unknown = null;

  const fire = (): void => {
    timer = null;
    if (!pending) return;
    const { value } = pending;
    pending = null;
    last = clock.now();
    send(value);
  };

  return {
    push(value: T): void {
      pending = { value };
      const wait = last + intervalMs - clock.now();
      if (wait <= 0) {
        if (timer !== null) {
          clock.clearTimeout(timer);
          timer = null;
        }
        fire();
      } else if (timer === null) {
        timer = clock.setTimeout(fire, wait);
      }
    },
    flush(): void {
      if (timer !== null) {
        clock.clearTimeout(timer);
        timer = null;
      }
      fire();
    },
    cancel(): void {
      if (timer !== null) clock.clearTimeout(timer);
      timer = null;
      pending = null;
    },
    get pending(): boolean {
      return pending !== null;
    },
  };
}

/** Wheel deltas accumulate instead of dropping: 30 tiny trackpad deltas in
 *  one interval become ONE wheel message with their sum, so scrolling keeps
 *  its distance while the message rate stays bounded. */
export interface WheelDelta {
  x: number;
  y: number;
  dx: number;
  dy: number;
  modifiers: number;
}

export function mergeWheel(a: WheelDelta | null, b: WheelDelta): WheelDelta {
  if (!a || a.modifiers !== b.modifiers) return b;
  return { x: b.x, y: b.y, dx: a.dx + b.dx, dy: a.dy + b.dy, modifiers: b.modifiers };
}

/** `WheelEvent.deltaMode` normalised to CSS px (Firefox reports lines). */
export function wheelPixels(delta: number, deltaMode: number, pageHeight: number): number {
  if (deltaMode === 1) return delta * 16; // DOM_DELTA_LINE
  if (deltaMode === 2) return delta * (pageHeight > 0 ? pageHeight : 800); // DOM_DELTA_PAGE
  return delta;
}
