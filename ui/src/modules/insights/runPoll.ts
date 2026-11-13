// Cadence of the Insights page's "Generating the report" poll. Pure, so
// `node --test` can load it.
//
// The first ~5 minutes poll every 3 s (most reports land then). A run can
// take up to 2 hours, so after that the banner stays and the poll slows to
// every 30 s for as long as the daemon still lists the run as active — it
// ends only when the report is ready or the run is gone (then the page says
// why), never on a timer that drops the banner while the agent still works.

/** Fast ticks before slowing down: 100 × 3 s ≈ 5 min. */
export const FAST_POLLS = 100;
export const FAST_POLL_MS = 3_000;
export const SLOW_POLL_MS = 30_000;

/** The wait before the next tick, given how many visible ticks already ran. */
export function runPollDelay(ticks: number): number {
  return ticks < FAST_POLLS ? FAST_POLL_MS : SLOW_POLL_MS;
}
