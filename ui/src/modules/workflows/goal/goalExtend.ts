// "Extend and resume…" (goal.md §3.5, GL7.2): the dialog proposes the caps
// +50 %, and never a limit at or below what is already used. Pure; unit-tested
// in ui/unit/goalExtend.test.ts.
import type { GoalProgress } from '../../../lib/api/types';
import { MAX_ITERATIONS } from './goalForm.ts';

const MIN_MINUTES = 15;
const MAX_MINUTES = 480;

/** New totals to propose: iterations and active minutes. */
export function extendProposal(goal: GoalProgress | undefined): { iterations: number; minutes: number } {
  const used = goal?.iteration ?? 0;
  const usedMin = Math.ceil((goal?.active_ms ?? 0) / 60_000);
  const iterations = Math.max(Math.ceil((goal?.max_iterations ?? 4) * 1.5), used + 1);
  const minutes = Math.max(Math.ceil(((goal?.max_active_ms ?? 3_600_000) * 1.5) / 60_000), usedMin + 15);
  return { iterations: Math.min(MAX_ITERATIONS, iterations), minutes: Math.min(MAX_MINUTES, minutes) };
}

/** Why the chosen totals cannot resume the run, or null. The dialog sends
 *  both, so both must leave room (the server's resume_limits refuses either). */
export function extendProblem(goal: GoalProgress | undefined, iterations: number, minutes: number): string | null {
  if (!goal) return null;
  if (!(iterations >= 1 && iterations <= MAX_ITERATIONS)) return `Choose 1 to ${MAX_ITERATIONS} iterations.`;
  if (!(minutes >= MIN_MINUTES && minutes <= MAX_MINUTES)) return `Choose ${MIN_MINUTES} to ${MAX_MINUTES} minutes of active time.`;
  if (iterations > goal.iteration && minutes * 60_000 > goal.active_ms) return null;
  return `Raise both limits above what is used: ${goal.iteration} iterations and ${Math.ceil(goal.active_ms / 60_000)} minutes.`;
}
