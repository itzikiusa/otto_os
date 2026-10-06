// Reconnect backoff for a Terminal socket (Terminal.svelte `scheduleReconnect`).
//
// Owners retry forever at 0.5 → 5 s. A guest (share link) used to stop after
// 8 refusals (~27 s), so a daemon restart that refused upgrades a little
// longer — a deploy, a reboot after an update — left a phone guest on a dead
// "Disconnected" terminal for good (S14-303). A lapsed/revoked share is
// refused forever, though, and a fast endless retry feeds the daemon's
// failure throttle (S1-05). So a guest gets the fast ladder for the first few
// refusals, then a slow 30 s retry for up to 10 minutes from the first
// refusal, then stops (the share page's minute access re-check — and the
// Reconnect buttons — still bring it back once the link answers again).

export const GUEST_FAST_REFUSALS = 8;
export const GUEST_SLOW_RETRY_MS = 30_000;
export const GUEST_RETRY_WINDOW_MS = 10 * 60_000;

/** Delay before the next reconnect attempt, or null to stop retrying. */
export function reconnectDelay(opts: {
  guest: boolean;
  /** Attempts scheduled so far in this outage (drives the fast ladder). */
  attempts: number;
  /** Guest upgrades refused in a row (never opened). */
  refusals: number;
  /** When the first of those refusals happened (ms epoch), else null. */
  firstRefusalAt: number | null;
  now: number;
}): number | null {
  const fast = Math.min(500 * 2 ** opts.attempts, 5000);
  if (!opts.guest || opts.refusals < GUEST_FAST_REFUSALS) return fast;
  const since = opts.firstRefusalAt === null ? 0 : opts.now - opts.firstRefusalAt;
  return since < GUEST_RETRY_WINDOW_MS ? GUEST_SLOW_RETRY_MS : null;
}
