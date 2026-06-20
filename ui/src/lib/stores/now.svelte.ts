// Shared 1-second clock for relative-time labels ("3m ago") that must tick
// without each component spinning its own interval. One interval drives a single
// reactive `ms`; reading `now()` or calling `rel(ts)` inside a template/$derived
// re-renders it every second. Mirrors the singleton-store idiom in
// viewport.svelte.ts. SSR-safe: no interval when `window` is absent.

class Clock {
  ms = $state(Date.now());

  constructor() {
    if (typeof window !== 'undefined') {
      setInterval(() => {
        this.ms = Date.now();
      }, 1000);
    }
  }
}

const clock = new Clock();

/** Reactive current epoch-ms. Call inside a reactive context to tick every 1s. */
export function now(): number {
  return clock.ms;
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
