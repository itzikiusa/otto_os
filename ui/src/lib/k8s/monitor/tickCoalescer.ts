// Coalesces `k8s_monitor_cycle` events into dashboard refresh ticks. Every
// monitor read is a ClickHouse aggregation, and with N clusters a cycle lands
// every interval/N seconds — so the views re-read at most once per `minGapMs`
// (the clusters that cycled meanwhile ride along), never while the document
// is hidden (one tick fires when it is visible again), and the first cycle
// after a quiet period ticks at once. Pure (clock / timer / visibility are
// injected) so it is unit-tested without a DOM.

export interface TickCoalescerEnv {
  now(): number;
  setTimer(fn: () => void, ms: number): unknown;
  hidden(): boolean;
  /** Call `fn` whenever the document becomes visible again (hooked once). */
  onVisible(fn: () => void): void;
}

export class TickCoalescer {
  private pending: string[] = [];
  private lastTickAt = Number.NEGATIVE_INFINITY;
  private timer = false;
  private hooked = false;

  private readonly minGapMs: number;
  private readonly emit: (clusters: string[]) => void;
  private readonly env: TickCoalescerEnv;

  constructor(minGapMs: number, emit: (clusters: string[]) => void, env: TickCoalescerEnv) {
    this.minGapMs = minGapMs;
    this.emit = emit;
    this.env = env;
  }

  /** A cluster finished a collection cycle. */
  cycle(clusterId: string): void {
    this.pending = [...this.pending.filter((c) => c !== clusterId), clusterId];
    this.schedule();
  }

  private schedule(): void {
    if (this.timer || !this.pending.length) return;
    if (this.env.hidden()) {
      this.hook();
      return;
    }
    const wait = Math.max(0, this.lastTickAt + this.minGapMs - this.env.now());
    this.timer = true;
    this.env.setTimer(() => {
      this.timer = false;
      this.fire();
    }, wait);
  }

  private fire(): void {
    if (!this.pending.length) return;
    if (this.env.hidden()) {
      this.hook();
      return;
    }
    this.lastTickAt = this.env.now();
    const clusters = this.pending;
    this.pending = [];
    this.emit(clusters);
  }

  private hook(): void {
    if (this.hooked) return;
    this.hooked = true;
    this.env.onVisible(() => this.schedule());
  }
}

/** The browser environment (setTimeout + document.visibilityState). */
export const browserTickEnv: TickCoalescerEnv = {
  now: () => Date.now(),
  setTimer: (fn, ms) => setTimeout(fn, ms),
  hidden: () => typeof document !== 'undefined' && document.visibilityState === 'hidden',
  onVisible: (fn) => {
    if (typeof document === 'undefined') return;
    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState !== 'hidden') fn();
    });
  },
};
