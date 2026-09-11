// A setTimeout CHAIN (not setInterval) for box auto-refresh: the next tick is
// scheduled only after the previous fetch settles, so a slow daemon can never
// stack overlapping requests, and consecutive failures back the cadence off
// (×2 per failure, capped ×8) so a broken backend isn't hammered at full rate.
// Ticks are skipped while the tab is hidden — a Home page left open overnight
// should not poll six boxes all night.

export interface Poller {
  stop(): void;
  /** Run immediately (manual refresh) and restart the cadence. */
  now(): void;
}

export function poll(run: () => Promise<boolean>, ms: number): Poller {
  let stopped = false;
  let failures = 0;
  let handle = 0;
  const schedule = (): void => {
    if (stopped) return;
    const backoff = Math.min(2 ** failures, 8);
    handle = window.setTimeout(() => void tick(), Math.max(ms, 5000) * backoff);
  };
  const tick = async (): Promise<void> => {
    if (stopped) return;
    if (typeof document !== 'undefined' && document.visibilityState === 'hidden') {
      schedule();
      return;
    }
    try {
      const ok = await run();
      failures = ok ? 0 : failures + 1;
    } catch {
      failures += 1;
    }
    schedule();
  };
  void tick();
  return {
    stop() {
      stopped = true;
      clearTimeout(handle);
    },
    now() {
      clearTimeout(handle);
      failures = 0;
      void tick();
    },
  };
}
