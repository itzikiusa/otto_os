// Window-wide resize-compact arbitration (perf F1). An agent TUI pane asks the
// daemon for a full snapshot (up to 4000 rows, 1–2 MB, ~150–350 ms of xterm
// parsing on the main thread) after a confirmed resize. Resizing the window or
// toggling a split with N visible agent panes used to fire N of them at once.
// Mirrors the GPU-slot arbitration in Terminal.svelte: ONE compact in flight
// per window, and the most recently focused waiting pane goes first. A pane
// that is off-screen or in a hidden window never queues — it marks itself and
// compacts when it comes back into view (Terminal.svelte `onCompactWake`).
// Kept free of xterm/DOM imports so the rules are unit-testable
// (unit/termCompactQueue.test.ts).

export interface CompactClient {
  /** `performance.now()` of the pane's last focus (0 = never). */
  lastFocus(): number;
  /** Still worth compacting now (connected, on screen, window visible). */
  eligible(): boolean;
  /** Send the snapshot request. Returns false when the pane declined at the
   *  last moment (reading scrollback, a selection): the slot frees at once. */
  run(): boolean;
}

/** An in-flight compact whose reply never came (socket died mid-request)
 *  frees the slot after this long. */
export const COMPACT_SLOT_TIMEOUT_MS = 5000;

type TimerId = ReturnType<typeof setTimeout>;

export class CompactQueue {
  private readonly waiting = new Set<CompactClient>();
  private inflight: CompactClient | null = null;
  private timer: TimerId | null = null;
  private readonly timeout: number;
  private readonly setT: (fn: () => void, ms: number) => TimerId;
  private readonly clearT: (id: TimerId) => void;

  constructor(
    timeout = COMPACT_SLOT_TIMEOUT_MS,
    setT: (fn: () => void, ms: number) => TimerId = (fn, ms) => setTimeout(fn, ms),
    clearT: (id: TimerId) => void = (id) => clearTimeout(id),
  ) {
    this.timeout = timeout;
    this.setT = setT;
    this.clearT = clearT;
  }

  /** The client currently holding the slot (tests / diagnostics). */
  get active(): CompactClient | null {
    return this.inflight;
  }

  get pending(): number {
    return this.waiting.size;
  }

  /** Queue `c` for one compact (idempotent while queued or in flight). */
  request(c: CompactClient): void {
    if (this.inflight === c) return;
    this.waiting.add(c);
    this.pump();
  }

  /** `c`'s snapshot reply arrived (any `scrollback`): free the slot. */
  done(c: CompactClient): void {
    if (this.inflight !== c) return;
    this.release();
    this.pump();
  }

  /** `c` went away (socket closed, parked, unmounted): forget it. */
  cancel(c: CompactClient): void {
    this.waiting.delete(c);
    this.done(c);
  }

  private release(): void {
    this.inflight = null;
    if (this.timer !== null) this.clearT(this.timer);
    this.timer = null;
  }

  /** Start the best eligible waiting client when the slot is free.
   *  Ineligible ones are dropped: they re-request when they come back. */
  pump(): void {
    while (this.inflight === null && this.waiting.size > 0) {
      let best: CompactClient | null = null;
      for (const c of [...this.waiting]) {
        if (!c.eligible()) {
          this.waiting.delete(c);
          continue;
        }
        if (!best || c.lastFocus() > best.lastFocus()) best = c;
      }
      if (!best) return;
      this.waiting.delete(best);
      this.inflight = best;
      const mine = best;
      this.timer = this.setT(() => {
        if (this.inflight !== mine) return;
        this.timer = null;
        this.inflight = null;
        this.pump();
      }, this.timeout);
      if (!best.run()) this.release();
    }
  }
}
