// The daemon's idle-suspend policy, as the Agents pane's "suspends in N" hint
// needs it. The sweep lives in crates/otto-sessions/src/manager.rs
// (`suspend_idle_unattached`); the defaults below MUST mirror its constants:
//   SUSPEND_GRACE           300 s  → setting `idle_suspend_grace_secs`
//   MANUAL_IDLE_SUSPEND   86400 s  → setting `manual_idle_suspend_secs` (0 = never)
//   MAX_LIVE_AGENT_SESSIONS   12   → setting `max_live_agent_sessions`  (0 = no cap)
// The hint is informational: the daemon also holds a session while anyone
// watches it, while a turn is open or its process tree is busy, and gives a
// passively resumed process (reopened, never typed into) only the engine grace.

export const DEFAULT_ENGINE_GRACE_SECS = 300;
export const DEFAULT_MANUAL_GRACE_SECS = 86_400;
export const DEFAULT_MAX_LIVE_AGENT_SESSIONS = 12;

export interface IdleSuspendPolicy {
  /** Quiet time before an engine-owned session is suspended. */
  engineGraceSecs: number;
  /** Quiet time before a session the user started is suspended; 0 = never. */
  manualGraceSecs: number;
  /** Soft cap on live agent CLIs; 0 = none. */
  maxLiveAgentSessions: number;
}

export const DEFAULT_IDLE_SUSPEND_POLICY: IdleSuspendPolicy = {
  engineGraceSecs: DEFAULT_ENGINE_GRACE_SECS,
  manualGraceSecs: DEFAULT_MANUAL_GRACE_SECS,
  maxLiveAgentSessions: DEFAULT_MAX_LIVE_AGENT_SESSIONS,
};

/** A non-negative integer setting, or the default when unset / not a number
 *  (the daemon reads them with `as_u64`, so anything else is "unset"). */
function u64(v: unknown, fallback: number): number {
  return typeof v === 'number' && Number.isInteger(v) && v >= 0 ? v : fallback;
}

/** The policy from a `GET /settings` body (missing keys → daemon defaults). */
export function policyFromSettings(all: Record<string, unknown> | null | undefined): IdleSuspendPolicy {
  return {
    engineGraceSecs: u64(all?.idle_suspend_grace_secs, DEFAULT_ENGINE_GRACE_SECS),
    manualGraceSecs: u64(all?.manual_idle_suspend_secs, DEFAULT_MANUAL_GRACE_SECS),
    maxLiveAgentSessions: u64(all?.max_live_agent_sessions, DEFAULT_MAX_LIVE_AGENT_SESSIONS),
  };
}

/** "26m" under 90 minutes, "23h" under two days, else "3d" — rounded UP, so
 *  a countdown never claims less time than is left. */
export function formatLeft(ms: number): string {
  const min = Math.ceil(ms / 60_000);
  if (min < 90) return `${min}m`;
  const h = Math.ceil(ms / 3_600_000);
  if (h < 48) return `${h}h`;
  return `${Math.ceil(ms / 86_400_000)}d`;
}

/** A grace as prose: "30 min", "4 h", "24 h", "3 days". */
export function formatGrace(secs: number): string {
  const min = Math.max(1, Math.round(secs / 60));
  if (min < 120) return `${min} min`;
  const h = Math.round(min / 60);
  if (h <= 48) return `${h} h`;
  const d = Math.round(h / 24);
  return `${d} days`;
}

export interface SuspendHint {
  label: string;
  title: string;
}

/** The countdown only shows inside this window before the suspend. Further
 *  out it is noise — and false for any pane someone is looking at, which the
 *  sweep never suspends (`is_watched` in manager.rs). */
export const COUNTDOWN_WINDOW_MS = 2 * 60 * 60 * 1000;

/**
 * "4m idle · suspends in 26m" for an idle agent pane — the countdown only when
 * the suspend is under {@link COUNTDOWN_WINDOW_MS} away, plain "4h idle"
 * before that (the full rule stays in `title`) — or null when the sweep would
 * never suspend it (a user-started session with the manual grace off).
 * `userStarted` mirrors `is_user_started` in manager.rs.
 */
export function suspendHint(idleMs: number, userStarted: boolean, policy: IdleSuspendPolicy): SuspendHint | null {
  if (idleMs < 0) return null;
  const graceSecs = userStarted ? policy.manualGraceSecs : policy.engineGraceSecs;
  if (userStarted && graceSecs === 0) return null;
  const idleMin = Math.floor(idleMs / 60_000);
  const idleSec = Math.floor((idleMs % 60_000) / 1000);
  const idleLabel =
    idleMin >= 90 ? `${Math.floor(idleMin / 60)}h idle` : idleMin > 0 ? `${idleMin}m idle` : `${idleSec}s idle`;
  const leftMs = graceSecs * 1000 - idleMs;
  const label =
    leftMs <= 0
      ? `${idleLabel} · suspending…`
      : leftMs < COUNTDOWN_WINDOW_MS
        ? `${idleLabel} · suspends in ${formatLeft(leftMs)}`
        : idleLabel;
  const cap = policy.maxLiveAgentSessions;
  const title =
    `Session is idle. Once nobody is watching it, auto-suspend frees its RAM after ${formatGrace(graceSecs)} of quiet ` +
    `(${userStarted ? 'sessions you start' : 'background sessions'}) while keeping it resumable.` +
    (cap > 0 ? ` With more than ${cap} live agent sessions, the least recently used idle ones may be suspended sooner.` : '');
  return { label, title };
}
