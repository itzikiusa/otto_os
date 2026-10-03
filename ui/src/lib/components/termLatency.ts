// Terminal latency probe (review 01 L1) — opt-in diagnostics that split
// keystroke-to-screen time into the segments a slow Mac can blame:
//
//   wire   keystroke sent → first output frame back (WS + daemon + child)
//   rtt    `probe` → `probe_ack` round trip (WS + daemon loop, NO child)
//   echo   daemon-side: input reached the PTY → the child's first output
//   parse  that frame handed to xterm → its write callback (setTimeout-gated)
//   paint  parsed → the next xterm render pass (rAF-gated)
//   drift  a 1 s setInterval's lateness (timer throttling / busy main thread)
//   rAF    animation-frame interval (paused/clamped when WebKit thinks the
//          window is hidden)
//
// Reading it: high echo = the CLI or its launchd QoS; low rtt but high
// parse/paint/drift = webview throttling or a busy main thread; high rtt =
// the daemon loop. Enable with `localStorage['otto.debug.termLatency']='1'`
// and reopen the pane; every terminal then shows a corner HUD.
//
// No DOM access at import time: the pure parts are unit-tested under node.

import type { TermEchoStats } from '../api/types';

/** localStorage key that turns the HUD on. */
export const TERM_LATENCY_FLAG = 'otto.debug.termLatency';

export function termLatencyEnabled(): boolean {
  try {
    return globalThis.localStorage?.getItem(TERM_LATENCY_FLAG) === '1';
  } catch {
    return false;
  }
}

/** Last N samples with percentiles (N small: computed once a second). */
export class RollingStats {
  private buf: number[] = [];
  private readonly cap: number;
  constructor(cap = 64) {
    this.cap = cap;
  }

  push(ms: number): void {
    if (!Number.isFinite(ms) || ms < 0) return;
    this.buf.push(ms);
    if (this.buf.length > this.cap) this.buf.shift();
  }

  get count(): number {
    return this.buf.length;
  }

  get last(): number | null {
    return this.buf.length ? this.buf[this.buf.length - 1] : null;
  }

  /** Nearest-rank percentile, `p` in 0..100; null with no samples. */
  pct(p: number): number | null {
    if (!this.buf.length) return null;
    const sorted = [...this.buf].sort((a, b) => a - b);
    const rank = Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1));
    return sorted[rank];
  }

  clear(): void {
    this.buf = [];
  }
}

/**
 * Per-terminal keystroke timeline. One sample in flight at a time: a burst of
 * keys measures from its first key until the frame that answers it paints.
 * Times are `performance.now()` values passed in (testable without a clock).
 */
export class KeyLatency {
  readonly wire = new RollingStats();
  readonly parse = new RollingStats();
  readonly paint = new RollingStats();
  private t0: number | null = null;
  /** The answering frame is in xterm: waiting for its parse callback. */
  private t1: number | null = null;
  /** Parsed: waiting for the next render pass. */
  private t2: number | null = null;

  /** A user keystroke left for the PTY. */
  input(now: number): void {
    if (this.t0 === null && this.t1 === null && this.t2 === null) this.t0 = now;
  }

  /** An output frame arrived. Returns a parse callback when this frame is the
   *  one answering a pending keystroke (pass it to the write queue). */
  frame(now: number, clock: () => number): (() => void) | undefined {
    if (this.t0 === null) return undefined;
    this.wire.push(now - this.t0);
    this.t0 = null;
    this.t1 = now;
    return () => this.parsed(clock());
  }

  parsed(now: number): void {
    if (this.t1 === null) return;
    this.parse.push(now - this.t1);
    this.t1 = null;
    this.t2 = now;
  }

  /** xterm finished a render pass. */
  rendered(now: number): void {
    if (this.t2 === null) return;
    this.paint.push(now - this.t2);
    this.t2 = null;
  }

  /** Drop a sample whose frame never came (socket swap, snapshot rebuild). */
  reset(): void {
    this.t0 = this.t1 = this.t2 = null;
  }
}

/** `probe` → `probe_ack` round trips, keyed by id. */
export class ProbeClock {
  readonly rtt = new RollingStats(32);
  private seq = 0;
  private sent = new Map<number, number>();

  /** Next probe id, stamped at `now`. Unanswered probes older than 30 s are
   *  forgotten (a dead socket never acks). */
  next(now: number): number {
    for (const [id, t] of this.sent) if (now - t > 30_000) this.sent.delete(id);
    const id = ++this.seq;
    this.sent.set(id, now);
    return id;
  }

  ack(id: number, now: number): void {
    const t = this.sent.get(id);
    if (t === undefined) return;
    this.sent.delete(id);
    this.rtt.push(now - t);
  }

  /** Probes still waiting for an ack (a stuck daemon loop shows here). */
  get outstanding(): number {
    return this.sent.size;
  }
}

/** Daemon-side keystroke echo stats from `probe_ack.echo`. */
export type EchoStats = TermEchoStats;

/**
 * Window-wide event-loop monitor, shared by every HUD (ref-counted): timer
 * drift of a 1 s interval, rAF interval, visibility and long tasks.
 */
class LoopMonitor {
  readonly drift = new RollingStats(30);
  readonly raf = new RollingStats(120);
  longTasks = 0;
  private refs = 0;
  private timer: ReturnType<typeof setInterval> | null = null;
  private rafId: number | null = null;
  private lastTick = 0;
  private lastRaf = 0;
  private observer: PerformanceObserver | null = null;

  acquire(): () => void {
    if (this.refs++ === 0) this.start();
    let released = false;
    return () => {
      if (released) return;
      released = true;
      if (--this.refs === 0) this.stop();
    };
  }

  private start(): void {
    this.lastTick = performance.now();
    this.timer = setInterval(() => { // ui-guards: allow — measures timer throttling, so it must run while hidden
      const now = performance.now();
      this.drift.push(Math.max(0, now - this.lastTick - 1000));
      this.lastTick = now;
    }, 1000);
    const frame = (t: number) => {
      if (this.lastRaf) this.raf.push(t - this.lastRaf);
      this.lastRaf = t;
      this.rafId = requestAnimationFrame(frame);
    };
    this.rafId = requestAnimationFrame(frame);
    try {
      if (PerformanceObserver.supportedEntryTypes?.includes('longtask')) {
        this.observer = new PerformanceObserver((list) => {
          this.longTasks += list.getEntries().length;
        });
        this.observer.observe({ type: 'longtask', buffered: false });
      }
    } catch {
      this.observer = null; // WebKit: no Long Tasks API
    }
  }

  private stop(): void {
    if (this.timer !== null) clearInterval(this.timer);
    if (this.rafId !== null) cancelAnimationFrame(this.rafId);
    this.observer?.disconnect();
    this.timer = this.rafId = this.observer = null;
    this.lastRaf = 0;
  }

  /** Long tasks are only countable where the engine supports the API. */
  get longTaskSupported(): boolean {
    return this.observer !== null;
  }
}

export const loopMonitor = new LoopMonitor();

/** `12`, `1.4` or `–` — HUD number formatting. */
export function fmtMs(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms)) return '–';
  return ms >= 10 ? String(Math.round(ms)) : ms.toFixed(1);
}

/** `p50/p95` of a stats window, e.g. `4.2/18`. */
export function fmtPair(s: RollingStats): string {
  return s.count ? `${fmtMs(s.pct(50))}/${fmtMs(s.pct(95))}` : '–';
}
