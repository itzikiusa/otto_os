// Shared 1-second clock for relative-time labels ("3m ago") that must tick
// without each component spinning its own interval. One interval drives a single
// reactive `ms`; reading `now()` or calling `rel(ts)` inside a template/$derived
// re-renders it every second. Mirrors the singleton-store idiom in
// viewport.svelte.ts. SSR-safe: no interval when `window` is absent.

class Clock {
  ms = $state(Date.now());
  /** Changes once a minute (A3): readers of minute-grain labels ("4h idle")
   *  re-run 60× less often than on the 1-second tick. */
  minuteMs = $state(Math.floor(Date.now() / 60_000) * 60_000);

  private tick(): void {
    this.ms = Date.now();
    const m = Math.floor(this.ms / 60_000) * 60_000;
    if (m !== this.minuteMs) this.minuteMs = m;
  }

  constructor() {
    if (typeof window !== 'undefined') {
      // A hidden window paints nothing, so its labels needn't tick: skip the
      // write (every reader's re-run) and catch up once on becoming visible.
      setInterval(() => {
        if (typeof document !== 'undefined' && document.hidden) return;
        this.tick();
      }, 1000);
      if (typeof document !== 'undefined') {
        document.addEventListener('visibilitychange', () => {
          if (!document.hidden) this.tick();
        });
      }
    }
  }
}

const clock = new Clock();

/** Reactive current epoch-ms. Call inside a reactive context to tick every 1s. */
export function now(): number {
  return clock.ms;
}

/** Reactive epoch-ms floored to the minute: re-evaluates once a minute. Use it
 *  where a label only changes per minute, so a dozen panes don't recompute
 *  every second. */
export function nowMinute(): number {
  return clock.minuteMs;
}

function toMs(ts: number | string | Date): number {
  if (typeof ts === 'number') return ts;
  if (ts instanceof Date) return ts.getTime();
  const n = Date.parse(ts);
  return Number.isNaN(n) ? 0 : n;
}

/**
 * Compact relative-time label vs the shared clock: "now", "5s", "3m", "2h",
 * "4d", or an absolute date beyond ~30 days. Past deltas get "ago"; future
 * deltas get "in …". Reactive — re-evaluates as the clock ticks.
 */
export function rel(ts: number | string | Date): string {
  const t = toMs(ts);
  if (!t) return '';
  const diff = clock.ms - t;
  const future = diff < 0;
  const s = Math.floor(Math.abs(diff) / 1000);
  let body: string;
  if (s < 5) return 'now';
  if (s < 60) body = `${s}s`;
  else if (s < 3600) body = `${Math.floor(s / 60)}m`;
  else if (s < 86400) body = `${Math.floor(s / 3600)}h`;
  else if (s < 2592000) body = `${Math.floor(s / 86400)}d`;
  else return new Date(t).toLocaleDateString();
  return future ? `in ${body}` : `${body} ago`;
}
