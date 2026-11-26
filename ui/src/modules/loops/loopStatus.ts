import type { GoalLoop } from '../../lib/api/types';
import type { StatusInfo } from '../../lib/status.ts';

// A goal loop's lifecycle status in the shared status vocabulary (lib/status.ts,
// patterns.md §1): same tones as a workflow/swarm run (running → info + live,
// failed → danger, stopped → neutral like a cancelled run), but the loop keeps
// its own words — "Blocked" and "Exhausted" tell the user what to do next,
// which the generic "Waiting" would hide.
const LOOP_STATUS: Record<string, Omit<StatusInfo, 'key'>> = {
  draft: { label: 'Draft', tone: 'neutral', hint: 'Not started yet' },
  running: { label: 'Running', tone: 'info', live: true, hint: 'Agents are iterating toward the goal' },
  paused: { label: 'Paused', tone: 'neutral', hint: 'Paused — continue it as a goal' },
  blocked: { label: 'Blocked', tone: 'warning', hint: 'Stopped on an open decision — continue it as a goal' },
  succeeded: { label: 'Succeeded', tone: 'success', hint: 'Every acceptance criterion is met' },
  exhausted: { label: 'Exhausted', tone: 'warning', hint: 'Hit its iteration or runtime budget before the goal was met' },
  failed: { label: 'Failed', tone: 'danger' },
  stopped: { label: 'Stopped', tone: 'neutral', hint: 'Stopped, or continued as a goal' },
};

/** An iteration's status once evaluated is what the evaluation decided: the
 *  run finishing is not the goal being met ("Succeeded · 55% · continuing"
 *  read as a win next to criteria that read "Not met"). Before an
 *  evaluation it is the run's own status. */
export function iterationStatus(run: StatusInfo, verdict: string | null | undefined): StatusInfo {
  if (verdict === 'achieved') return { key: 'met', label: 'Goal met', tone: 'success' };
  if (verdict === 'continue') return { key: 'continuing', label: 'Continuing', tone: 'neutral', hint: 'The goal is not met yet; the next iteration carries on' };
  if (verdict === 'blocked') return { key: 'blocked', label: 'Blocked', tone: 'warning', hint: 'Needs you: answer the open decision, then Resume' };
  return run;
}

/** Boot recovery stores "Interrupted by a daemon restart — …" as the error of
 *  every loop it pauses; a person's Pause never sets one. */
export function pausedByRestart(loop: Pick<GoalLoop, 'status' | 'error'>): boolean {
  return loop.status === 'paused' && /daemon restart/i.test(loop.error ?? '');
}

/** The loop's status, with a restart pause told apart from a person's. */
export function loopStatusOf(loop: Pick<GoalLoop, 'status' | 'error'>): StatusInfo {
  if (pausedByRestart(loop)) return { key: 'paused-restart', label: 'Paused by restart', tone: 'warning', hint: 'Otto restarted while this loop was running. Its work is kept — continue it as a goal.' };
  return loopStatus(loop.status);
}

/** Mirror of `VERIFY_OUTPUT_MARKER` in crates/otto-workflows/src/criteria.rs:
 *  a command criterion's evidence is "<exit status>" + marker + the output's tail. */
export const VERIFY_OUTPUT_MARKER = '\n--- verify output ---\n';

/** Split stored evidence into its headline and the verify command's output. */
export function splitEvidence(evidence: string): { headline: string; output: string } {
  const at = evidence.indexOf(VERIFY_OUTPUT_MARKER);
  if (at < 0) return { headline: evidence, output: '' };
  return { headline: evidence.slice(0, at), output: evidence.slice(at + VERIFY_OUTPUT_MARKER.length) };
}

/** A loop a person may continue as a goal: not finished, not continued yet
 *  (Continue writes it `stopped`). */
export function canContinue(loop: Pick<GoalLoop, 'status'>): boolean {
  return ['draft', 'paused', 'blocked', 'exhausted'].includes(loop.status);
}

export function loopStatus(raw: string | null | undefined): StatusInfo {
  const k = (raw ?? '').trim().toLowerCase();
  const info = LOOP_STATUS[k];
  if (info) return { key: k, ...info };
  return { key: k || 'unknown', label: k ? k[0].toUpperCase() + k.slice(1) : 'Unknown', tone: 'neutral' };
}
