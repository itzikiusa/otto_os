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
// Staleness: Chromium only sends a screencast frame when the page CHANGES,
// so a quiet page is not a stale one — and the live socket has no heartbeat
// to tell a quiet page from a wedged pipe. The picture is therefore dimmed
// only when the socket itself isn't live (connecting / reconnecting / ended).
// `input` marks when the viewer last acted, so the next frame's arrival can
// be measured as input-to-frame latency.

export type LiveStatus = 'idle' | 'connecting' | 'live' | 'reconnecting' | 'ended';

export interface ConnState {
  status: LiveStatus;
  /** Consecutive failed attempts since the last successful open. */
  attempt: number;
  /** ms until the next retry while `reconnecting`, else 0. */
  retryInMs: number;
  /** Why it ended / what the last failure was — human-readable. */
  reason: string;
  /** Wall-clock ms of the last frame (0 = none yet). */
  lastFrameAt: number;
  /** Wall-clock ms of the first input sent since the last frame (0 = none
   *  pending) — the start of a latency sample. */
  awaitingSince: number;
  /** True once any frame arrived on this connection generation. */
  hasFrame: boolean;
}

export type ConnEvent =
  | { type: 'connect' }
  | { type: 'open' }
  | { type: 'frame'; at: number }
  | { type: 'input'; at: number }
  | { type: 'close'; code: number; reason?: string }
  | { type: 'retry' }
  | { type: 'ended'; reason: string }
  | { type: 'reset' };

export const INITIAL: ConnState = {
  status: 'idle',
  attempt: 0,
  retryInMs: 0,
  reason: '',
  lastFrameAt: 0,
  awaitingSince: 0,
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
      return { ...s, status: 'live', attempt: 0, retryInMs: 0, reason: '', awaitingSince: 0 };
    case 'frame':
      if (s.status !== 'live') return s;
      return { ...s, lastFrameAt: ev.at, awaitingSince: 0, hasFrame: true };
    case 'input':
      if (s.status !== 'live' || s.awaitingSince) return s;
      return { ...s, awaitingSince: ev.at };
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

/** Whether the picture on screen should be dimmed as out of date. */
export function isStale(s: ConnState): boolean {
  return s.status === 'reconnecting' || s.status === 'connecting' || s.status === 'ended';
}

/** Only frames within this long of an input count as that input's answer. */
export const LATENCY_WINDOW_MS = 1500;

/** Input-to-frame latency sample for a frame arriving at `at`, or null when
 *  no input was waiting (or it waited too long to be this frame's cause). */
export function latencySample(s: ConnState, at: number): number | null {
  if (!s.awaitingSince) return null;
  const d = at - s.awaitingSince;
  return d >= 0 && d <= LATENCY_WINDOW_MS ? d : null;
}

// ── fps / latency meter ─────────────────────────────────────────────────
// A tiny ring of recent frame arrival times → frames per second over the
// last second, and an EWMA of input-to-frame latency (time from sending an
// input to the next frame drawn — what the viewer actually feels). Both feed
// the subtle indicator in the live view's corner.

export interface Meter {
  frames: number[];
  latencyMs: number | null;
}

export const EMPTY_METER: Meter = { frames: [], latencyMs: null };

export function meterFrame(m: Meter, at: number): Meter {
  const cutoff = at - 1000;
  const frames = m.frames.filter((t) => t > cutoff);
  frames.push(at);
  return { ...m, frames };
}

export function meterFps(m: Meter, now: number): number {
  return m.frames.filter((t) => t > now - 1000).length;
}

export function meterLatency(m: Meter, sample: number): Meter {
  const latencyMs = m.latencyMs === null ? sample : Math.round(m.latencyMs * 0.7 + sample * 0.3);
  return { ...m, latencyMs };
}
