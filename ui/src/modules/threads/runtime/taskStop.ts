// Stopping a runtime thread's background work (docs/contracts/api.md
// "POST /runtime-sessions/{id}/tasks/{task_id}/stop"): what a row says while
// a stop is in flight, after the agent was only ASKED to stop, and when it
// failed — merged from the daemon's own `stopping` / `stop_error` and this
// tab's local answer. Never pretends: a row leaves "Running" only when the
// task leaves the daemon's list. Pure (unit/threadsTaskStop.test.ts).
import type { BackgroundTask, RuntimeTaskStopOutcome } from '../../../lib/api/types.ts';
import { plural } from '../../../lib/plural.ts';

export type StopPhase = 'stopping' | 'asked' | 'error' | 'gone';

export interface StopState {
  phase: StopPhase;
  /** Why it failed (`error`). */
  error?: string;
}

/** What a stop call's error means. `unsupported`: this daemon has no stop
 *  route (an older build answers its "no such route" 404) — hide every Stop
 *  rather than show a button that can never work. A 404 for the task, or a
 *  409, means it already ended. */
export type StopFailure = { unsupported: true } | { unsupported: false; state: StopState };

export function stopFailure(e: unknown): StopFailure {
  const status = typeof e === 'object' && e !== null ? (e as { status?: unknown }).status : undefined;
  const message = e instanceof Error ? e.message : typeof e === 'string' ? e : '';
  if (status === 404 && (/no such route/i.test(message) || !message)) return { unsupported: true };
  if (status === 404 || status === 409) return { unsupported: false, state: { phase: 'gone' } };
  return { unsupported: false, state: { phase: 'error', error: message.trim() || 'The daemon didn’t answer. Try again.' } };
}

/** A stop the daemon accepted: a native or signal stop is "Stopping…" until
 *  the provider settles the task; `asked` only asked the agent. */
export function stopStateOf(outcome: RuntimeTaskStopOutcome): StopState {
  return { phase: outcome === 'asked' ? 'asked' : 'stopping' };
}

export interface RowStop {
  /** Show the Stop (or Retry) button. */
  canStop: boolean;
  /** A stop is in flight: the button is replaced by "Stopping…". */
  busy: boolean;
  /** The row's stop line, or null. */
  note: string | null;
  /** The note is a failure (danger tone, the button reads Retry). */
  failed: boolean;
}

/** One running row's stop affordance. `local` is this tab's last answer,
 *  `server` the daemon's fields on the task (newer daemons). */
export function rowStop(
  server: Pick<BackgroundTask, 'stopping' | 'stop_error'> | null | undefined,
  local: StopState | null | undefined,
  unsupported: boolean,
): RowStop {
  if (unsupported) return { canStop: false, busy: false, note: null, failed: false };
  if (local?.phase === 'gone') return { canStop: false, busy: false, note: 'Already ended', failed: false };
  if (local?.phase === 'error') return { canStop: true, busy: false, note: `Couldn’t stop: ${local.error ?? 'unknown reason'}`, failed: true };
  if (local?.phase === 'asked' && !server?.stop_error) {
    return { canStop: false, busy: false, note: 'Asked the agent to stop it', failed: false };
  }
  if (server?.stop_error) return { canStop: true, busy: false, note: `Couldn’t stop: ${server.stop_error}`, failed: true };
  if (local?.phase === 'stopping' || server?.stopping) return { canStop: false, busy: true, note: 'Stopping…', failed: false };
  return { canStop: true, busy: false, note: null, failed: false };
}

/** The toast after "Stop all" / a child card's Stop, in words. */
export function stopSummary(r: { stopping: number; asked: number; failed: number; turn?: boolean }): { ok: boolean; title: string; body?: string } {
  const parts: string[] = [];
  if (r.turn) parts.push('Stopped the turn');
  if (r.stopping) parts.push(`Stopping ${plural(r.stopping, 'background task')}`);
  if (r.asked) parts.push(`Asked the agent to stop ${plural(r.asked, 'task')}`);
  if (r.failed) {
    return {
      ok: false,
      title: `Couldn’t stop ${plural(r.failed, 'background task')}`,
      body: parts.length ? `${parts.join(' · ')}. The rest are listed in Agents & tasks with the reason.` : 'Agents & tasks lists each one with the reason and Retry.',
    };
  }
  return { ok: true, title: parts.join(' · ') || 'Nothing was running' };
}

/** A Threads row whose agent is idle but whose background work runs: its
 *  own calm state (the `running` dot, no pulse), never "Working". */
export function withBackgroundState<T extends { key: string; label: string; hint?: string; live?: boolean }>(st: T, backgroundTasks: number): T {
  if (st.key !== 'running' || backgroundTasks <= 0) return st;
  return { ...st, label: 'Background task running', hint: `${backgroundLine(backgroundTasks)} — the agent is idle`, live: false };
}

/** "3 background tasks running" — the composer's line on a thread whose turn ended. */
export function backgroundLine(n: number): string {
  return `${plural(n, 'background task')} running`;
}
