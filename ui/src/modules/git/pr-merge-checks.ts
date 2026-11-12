// The forge's own merge preconditions (`PrDetail.merge_checks`, Bitbucket
// merge checks / GitHub rules) and open tasks, in words (pure —
// unit/prMergeChecks.test.ts). The merge dialog already says "CI failing" from
// its own checks probe, so build/status checks are not repeated as reasons.
import type { PrDetail, PrMergeCheck } from '../../lib/api/types';
import { plural } from '../../lib/plural.ts';

/** "2 of 3 checks passed"; null when the forge reports none. */
export function mergeCheckSummary(checks: readonly PrMergeCheck[] | null | undefined): string | null {
  if (!checks || checks.length === 0) return null;
  const passed = checks.filter((c) => c.passed).length;
  if (passed < checks.length) return `${passed} of ${plural(checks.length, 'check')} passed`;
  return checks.length === 1 ? 'Check passed' : `All ${checks.length} checks passed`;
}

/** "3 tasks open"; null when none are open (or the forge has no tasks). */
export function openTasksLine(t: PrDetail['task_count']): string | null {
  return t && t.open > 0 ? `${plural(t.open, 'task')} open` : null;
}

/** A check about CI / builds — the merge dialog's CI row already covers it. */
const isCiCheck = (c: PrMergeCheck): boolean => /\b(ci|build|builds|status|statuses|check[_ -]?runs?)\b/i.test(c.key.replace(/[_-]/g, ' '));

/** Merge-dialog reasons from failing forge checks (CI ones left out) and open
 *  tasks (unless a failing check already says so). */
export function mergeCheckReasons(pr: Pick<PrDetail, 'merge_checks' | 'task_count'>): string[] {
  const failing = (pr.merge_checks ?? []).filter((c) => !c.passed && !isCiCheck(c));
  const tasks = failing.some((c) => /task/i.test(c.key)) ? null : openTasksLine(pr.task_count);
  const out = failing.map((c) => c.label);
  return tasks ? [...out, tasks] : out;
}
