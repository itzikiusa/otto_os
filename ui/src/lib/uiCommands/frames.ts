// Agent UI control — the pure half of the `/ws/events` UI protocol: frame
// validation, command-name normalisation, deadlines and the human labels the
// driving bar shows. No Svelte, no DOM, no runtime imports, so it runs under
// `node --test` (unit/uiCommands.test.ts). The runtime is lib/uiCommands.ts.

import type { UiAgentRef, UiCommandErrorCode, UiServerFrame } from '../api/types';

/** Largest result body the runtime posts back (the daemon caps it at 1 MiB). */
export const RESULT_MAX_BYTES = 1024 * 1024;
/** A command id the runtime already ran is ignored if the frame arrives again. */
export const SEEN_IDS_MAX = 200;
/** The longest the daemon waits on a person (a confirm), mirrored locally. */
export const HUMAN_WAIT_MS = 120_000;
/** How long the driving bar stays after the agent's last action. */
export const DRIVING_LINGER_MS = 60_000;
/** Recent actions kept for the bar's "Recent actions" menu. */
export const RECENT_MAX = 20;

export const ERROR_CODES: readonly UiCommandErrorCode[] = [
  'cancelled_by_user',
  'invalid_args',
  'not_found',
  'forbidden',
  'failed',
];

/** `otto.ui_db_run_query` / `otto_ui_db_run_query` / `ui_db_run_query` → `db_run_query`. */
export function normalizeCommand(name: string): string {
  return name.trim().replace(/^otto[._]/, '').replace(/^ui_/, '');
}

type Obj = Record<string, unknown>;
const isObj = (v: unknown): v is Obj => !!v && typeof v === 'object' && !Array.isArray(v);
const str = (v: unknown, max = 512): v is string => typeof v === 'string' && v.length > 0 && v.length <= max;

function readAgent(v: unknown, sessionId: string): UiAgentRef {
  const a = isObj(v) ? v : {};
  return {
    session_id: str(a.session_id, 128) ? a.session_id : sessionId,
    title: typeof a.title === 'string' ? a.title.slice(0, 200) : '',
    provider: typeof a.provider === 'string' ? a.provider.slice(0, 64) : '',
  };
}

/**
 * Validate a server frame that belongs to the UI protocol. Returns null for
 * anything else — including ordinary broadcast `OttoEvent`s, which the events
 * client then dispatches as before.
 */
export function readServerFrame(data: unknown): UiServerFrame | null {
  if (!isObj(data)) return null;
  switch (data.type) {
    case 'hello_ack':
      return str(data.conn_id, 128) ? { type: 'hello_ack', conn_id: data.conn_id } : null;
    case 'ui_command': {
      if (!str(data.id, 128) || !str(data.session_id, 128) || !str(data.command, 128)) return null;
      const deadline = typeof data.deadline_ms === 'number' && Number.isFinite(data.deadline_ms) ? data.deadline_ms : 30_000;
      return {
        type: 'ui_command',
        id: data.id,
        session_id: data.session_id,
        agent: readAgent(data.agent, data.session_id),
        command: normalizeCommand(data.command),
        args: isObj(data.args) ? data.args : {},
        deadline_ms: deadline,
      };
    }
    case 'ui_command_cancel':
      return str(data.id, 128)
        ? { type: 'ui_command_cancel', id: data.id, reason: typeof data.reason === 'string' ? data.reason.slice(0, 200) : '' }
        : null;
    default:
      return null;
  }
}

/** The frame's deadline as an absolute epoch-ms. `deadline_ms` is a duration;
 *  a value that already looks like an epoch (older/newer daemons) is kept. */
export function absoluteDeadline(deadlineMs: number, nowMs: number): number {
  if (deadlineMs > 1e12) return deadlineMs;
  return nowMs + Math.max(1000, Math.min(deadlineMs, HUMAN_WAIT_MS));
}

/** Module prefixes that a command name repeats (`db_run_query` → "Run query"). */
const PREFIXES = new Set([
  'db', 'api', 'git', 'browser', 'k8s', 'aws', 'brokers', 'broker', 'vault', 'workflows', 'workflow',
  'scheduled', 'sched', 'wf', 'home', 'swarm', 'loops', 'loop', 'nav',
]);

/** Commands whose name alone reads badly as an action. */
const LABELS: Record<string, string> = {
  state: 'Look at the window',
  open: 'Open a module',
  focus: 'Focus the window',
};

/** Sentence-case label for a command name, for the driving bar and its trail. */
export function commandLabel(name: string): string {
  const norm = normalizeCommand(name);
  if (LABELS[norm]) return LABELS[norm];
  const parts = norm.split('_').filter(Boolean);
  if (parts.length > 1 && PREFIXES.has(parts[0])) parts.shift();
  const text = parts.join(' ');
  return text ? text[0].toUpperCase() + text.slice(1) : 'Action';
}

/** "Claude" / "Codex" / "Antigravity" — the agent's display name from its provider. */
export function providerName(provider: string): string {
  const p = provider.trim().toLowerCase();
  if (p === '') return 'Agent';
  if (p === 'claude') return 'Claude';
  if (p === 'codex') return 'Codex';
  if (p === 'agy' || p === 'antigravity' || p === 'gemini') return 'Antigravity';
  return provider.trim()[0].toUpperCase() + provider.trim().slice(1);
}

/** Serialise a handler's result for `POST …/result`, or null when it can't be
 *  sent (not JSON-able, or over {@link RESULT_MAX_BYTES}). */
export function encodeResult(result: unknown): string | null {
  let body: string;
  try {
    body = JSON.stringify({ ok: true, result: result === undefined ? null : result });
  } catch {
    return null;
  }
  return new TextEncoder().encode(body).length > RESULT_MAX_BYTES ? null : body;
}

/**
 * How far to scroll a box [lo, hi] (the viewport of a scroller) so [a, b] (the
 * target) shows — by the nearest edge, never more than needed. A target
 * bigger than the box that already shows in part stays put (a tall result
 * grid must not shove the editor above it off screen).
 */
export function nearestDelta(a: number, b: number, lo: number, hi: number, pad = 8): number {
  const visible = b > lo && a < hi;
  if (a >= lo && b <= hi) return 0; // fully in view
  if (b - a > hi - lo) return visible ? 0 : a - lo - pad; // too big: show its start only if hidden
  if (a < lo) return a - lo - pad;
  return b - hi + pad;
}
