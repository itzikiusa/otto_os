// What a person needs to trust a loop that ran unattended: Otto's own per-run
// time cap, whether a restart (not a person) paused it, and the output a
// verify command printed. Pure helpers, no Svelte, so node:test covers them
// (unit/loopSafety.test.ts).
import type { GoalLoop } from '../../lib/api/types';
import type { StatusInfo } from '../../lib/status.ts';
import { loopStatus } from './loopStatus.ts';

/** Mirror of `MAX_RUN_ACTIVE_SECS` in crates/otto-automation/src/goal_loop.rs:
 *  one run (a start or Resume until it settles) stops after this much active
 *  time, whatever the loop's own runtime limit says. Not configurable. */
export const RUN_CAP_SECS = 4 * 60 * 60;

/** Mirror of `VERIFY_OUTPUT_MARKER` in goal_loop_policy.rs: a command
 *  criterion's evidence is "<exit status>" + marker + the output's tail. */
export const VERIFY_OUTPUT_MARKER = '\n--- verify output ---\n';

/** Boot recovery stores "Interrupted by a daemon restart — …" as the error of
 *  every loop it pauses; a person's Pause never sets one and Resume clears it. */
export function pausedByRestart(loop: Pick<GoalLoop, 'status' | 'error'>): boolean {
  return loop.status === 'paused' && /daemon restart/i.test(loop.error ?? '');
}

/** The loop's status, with a restart pause told apart from a person's. */
export function loopStatusOf(loop: Pick<GoalLoop, 'status' | 'error'>): StatusInfo {
  if (pausedByRestart(loop)) {
    return {
      key: 'paused-restart',
      label: 'Paused by restart',
      tone: 'warning',
      hint: 'Otto restarted while this loop was running. Its work is kept — Resume to continue.',
    };
  }
  return loopStatus(loop.status);
}

/** True when Otto's per-run cap, not the loop's own runtime limit, is what
 *  this run can use up first. */
export function runCapBinds(loop: Pick<GoalLoop, 'limits'>, elapsedSecs: number): boolean {
  return loop.limits.max_runtime_secs - elapsedSecs > RUN_CAP_SECS;
}

/** Split stored evidence into its headline and the verify command's output. */
export function splitEvidence(evidence: string): { headline: string; output: string } {
  const at = evidence.indexOf(VERIFY_OUTPUT_MARKER);
  if (at < 0) return { headline: evidence, output: '' };
  return { headline: evidence.slice(0, at), output: evidence.slice(at + VERIFY_OUTPUT_MARKER.length) };
}
