// Why "Launch loop" is disabled: the FIRST unmet condition, shown in place
// (the button used to name the name/criteria even when the Goal was missing).
import type { AcceptanceCriterion } from '../../lib/api/types';

export interface LaunchForm {
  name: string;
  seed: string;
  mode: 'build' | 'research';
  repoPath: string;
  graphIssue: string | null;
  graphTasks: number;
  maxIterations: number;
  maxMinutes: number;
  perPhaseMinutes: number;
  executorCount: number;
  criteria: readonly Pick<AcceptanceCriterion, 'text' | 'verify' | 'verify_kind' | 'verify_cmd'>[];
}

export function launchBlocker(f: LaunchForm): string | null {
  if (!f.seed.trim()) return 'Describe the goal.';
  if (f.mode === 'build' && !f.repoPath.trim()) return 'Pick the repository to work in.';
  if (!f.name.trim()) return 'Give the loop a name.';
  if (f.criteria.length === 0) return 'Add at least one acceptance criterion.';
  if (f.criteria.some((c) => !c.text.trim() || !c.verify.trim())) return 'Each criterion needs a description and how it is verified.';
  if (f.criteria.some((c) => c.verify_kind === 'command' && !(c.verify_cmd ?? '').trim())) return 'A command criterion needs its shell command.';
  const budget = [f.maxIterations, f.maxMinutes, f.perPhaseMinutes, f.executorCount];
  if (!budget.every((v) => Number.isInteger(v) && v > 0) || f.executorCount > 6) return 'Budget values must be whole numbers above 0, with at most 6 executors.';
  if (f.maxIterations < f.graphTasks) return 'Increase the iteration budget to cover every graph task.';
  if (f.graphIssue) return 'Fix the execution graph.';
  return null;
}
