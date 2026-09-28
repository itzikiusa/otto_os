// The daemon's idle-suspend policy, as the Agents pane's "suspends in N" hint
// needs it. The sweep lives in crates/otto-sessions/src/manager.rs
// (`suspend_idle_unattached`); the defaults below MUST mirror its constants:
//   SUSPEND_GRACE           300 s  → setting `idle_suspend_grace_secs`
//   MANUAL_IDLE_SUSPEND    1800 s  → setting `manual_idle_suspend_secs` (0 = never)
//   MAX_LIVE_AGENT_SESSIONS   12   → setting `max_live_agent_sessions`  (0 = no cap)
// The hint is informational: the daemon also holds a session while anyone
// watches it, while a turn is open or its process tree is busy, and gives a
// passively resumed process (reopened, never typed into) only the engine grace.

export const DEFAULT_ENGINE_GRACE_SECS = 300;
export const DEFAULT_MANUAL_GRACE_SECS = 1800;
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

export interface SuspendHint {
  label: string;
  title: string;
}

/**
 * "4m idle · suspends in 26m" for an idle agent pane, or null when the sweep
 * would never suspend it (a user-started session with the manual grace off).
 * `userStarted` mirrors `is_user_started` in manager.rs.
 */
export function suspendHint(idleMs: number, userStarted: boolean, policy: IdleSuspendPolicy): SuspendHint | null {
  if (idleMs < 0) return null;
  const graceSecs = userStarted ? policy.manualGraceSecs : policy.engineGraceSecs;
  if (userStarted && graceSecs === 0) return null;
  const idleMin = Math.floor(idleMs / 60_000);
  const idleSec = Math.floor((idleMs % 60_000) / 1000);
  const idleLabel = idleMin > 0 ? `${idleMin}m idle` : `${idleSec}s idle`;
  const leftMs = graceSecs * 1000 - idleMs;
  const label = leftMs <= 0 ? `${idleLabel} · suspending…` : `${idleLabel} · suspends in ${Math.ceil(leftMs / 60_000)}m`;
  const graceMin = Math.max(1, Math.round(graceSecs / 60));
  const cap = policy.maxLiveAgentSessions;
  const title =
    `Session is idle. Once nobody is watching it, auto-suspend frees its RAM after ${graceMin} min of quiet ` +
    `(${userStarted ? 'sessions you start' : 'background sessions'}) while keeping it resumable.` +
    (cap > 0 ? ` With more than ${cap} live agent sessions, the least recently used idle ones may be suspended sooner.` : '');
  return { label, title };
}
