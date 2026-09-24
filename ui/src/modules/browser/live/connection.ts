// Remote live browser — connection state machine. No imports (unit-tested).
//
// A pure reducer so every transition (and the backoff schedule) is testable
// without a socket:
//
//   idle ──connect──▶ connecting ──open──▶ live
//                        ▲   │                │ close (retryable)
//                        │   └─close──────────┤
//                        │                    ▼
//                        └──── retry ──── reconnecting (delay = backoff)
//
//   any ──ended──▶ ended      (the server said the session is over: tab
//                              closed, engine stopped, kicked — don't retry)
//   reconnecting ──give up──▶ ended (after maxAttempts)
//
// "live" also tracks freshness. Chromium only sends a screencast frame when
// the page CHANGES, so a quiet page is not a stale one: liveness comes from
// any sign of life — a frame or a ping reply (`heartbeat`). When neither
// arrived for STALE_MS the pipe is wedged and the view dims the last frame.

export type LiveStatus = 'idle' | 'connecting' | 'live' | 'reconnecting' | 'ended';

export interface ConnState {
  status: LiveStatus;
  /** Consecutive failed attempts since the last successful open. */
  attempt: number;
  /** ms until the next retry while `reconnecting`, else 0. */
  retryInMs: number;
  /** Why it ended / what the last failure was — human-readable. */
  reason: string;
  /** Wall-clock ms of the last sign of life — frame or pong (0 = none yet). */
  lastSeenAt: number;
  /** True once any frame arrived on this connection generation. */
  hasFrame: boolean;
}

export type ConnEvent =
  | { type: 'connect' }
  | { type: 'open' }
  | { type: 'frame'; at: number }
  | { type: 'heartbeat'; at: number }
  | { type: 'close'; code: number; reason?: string }
  | { type: 'retry' }
  | { type: 'ended'; reason: string }
  | { type: 'reset' };

export const INITIAL: ConnState = {
  status: 'idle',
  attempt: 0,
  retryInMs: 0,
  reason: '',
  lastSeenAt: 0,
  hasFrame: false,
};

export const BACKOFF = { baseMs: 500, maxMs: 15_000, factor: 2, maxAttempts: 8 };

/** Exponential backoff with a cap. `jitter` ∈ [0,1) spreads reconnect
 *  storms when the daemon restarts under many viewers (±20%). */
export function backoffDelay(attempt: number, jitter = 0.5, cfg = BACKOFF): number {
  const raw = Math.min(cfg.maxMs, cfg.baseMs * cfg.factor ** Math.max(0, attempt - 1));
  return Math.round(raw * (0.8 + 0.4 * jitter));
}

/** WebSocket close codes the server uses to say "don't come back":
 *  1000 normal (session closed), 1008 policy (auth/RBAC), 4000–4499 app-level
 *  terminal codes (4404 no such live session, 4403 forbidden, 4409 taken by
 *  another viewer). Everything else (1001 going away, 1006 abnormal, 1011,
 *  1012 restart, 1013 try again, 4500–4999 app-level transient) retries. */
export function isTerminalClose(code: number): boolean {
  return code === 1000 || code === 1008 || (code >= 4000 && code < 4500);
}

export function reduce(s: ConnState, ev: ConnEvent, jitter = 0.5): ConnState {
  switch (ev.type) {
    case 'reset':
      return INITIAL;
    case 'connect':
      if (s.status === 'connecting' || s.status === 'live') return s;
      return { ...s, status: 'connecting', retryInMs: 0, hasFrame: false };
    case 'open':
      if (s.status !== 'connecting') return s;
      return { ...s, status: 'live', attempt: 0, retryInMs: 0, reason: '', lastSeenAt: 0 };
    case 'frame':
      if (s.status !== 'live') return s;
      return { ...s, lastSeenAt: ev.at, hasFrame: true };
    case 'heartbeat':
      if (s.status !== 'live') return s;
      return { ...s, lastSeenAt: ev.at };
    case 'close': {
      if (s.status === 'ended' || s.status === 'idle') return s;
      if (isTerminalClose(ev.code)) {
        return { ...s, status: 'ended', retryInMs: 0, reason: ev.reason || 'The live session ended.' };
      }
      const attempt = s.attempt + 1;
      if (attempt > BACKOFF.maxAttempts) {
        return { ...s, status: 'ended', attempt, retryInMs: 0, reason: 'Lost the connection to Otto and could not reconnect.' };
      }
      return {
        ...s,
        status: 'reconnecting',
        attempt,
        retryInMs: backoffDelay(attempt, jitter),
        reason: ev.reason || 'Connection lost.',
      };
    }
    case 'retry':
      if (s.status !== 'reconnecting') return s;
      return { ...s, status: 'connecting', retryInMs: 0 };
    case 'ended':
      return { ...s, status: 'ended', retryInMs: 0, reason: ev.reason };
  }
}

export const STALE_MS = 4000;

/** Whether the picture on screen should be dimmed as out of date. */
export function isStale(s: ConnState, now: number): boolean {
  if (s.status === 'reconnecting' || s.status === 'connecting' || s.status === 'ended') return true;
  if (s.status !== 'live' || s.lastSeenAt === 0) return false;
  return now - s.lastSeenAt > STALE_MS;
}

// ── fps / latency meter ─────────────────────────────────────────────────
// A tiny ring of recent frame arrival times → frames per second over the
// last second, and an EWMA of ping round trips. Both feed the subtle
// indicator in the live view's status strip.

export interface Meter {
  frames: number[];
  rttMs: number | null;
}

export const EMPTY_METER: Meter = { frames: [], rttMs: null };

export function meterFrame(m: Meter, at: number): Meter {
  const cutoff = at - 1000;
  const frames = m.frames.filter((t) => t > cutoff);
  frames.push(at);
  return { ...m, frames };
}

export function meterFps(m: Meter, now: number): number {
  return m.frames.filter((t) => t > now - 1000).length;
}

export function meterRtt(m: Meter, sample: number): Meter {
  const rttMs = m.rttMs === null ? sample : Math.round(m.rttMs * 0.7 + sample * 0.3);
  return { ...m, rttMs };
}
