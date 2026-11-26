// The Optimize review page's words and choices (addendum-chat-optimize.md
// §2.4): changes grouped by what they may change, the first selection from
// the server's `preselected`, the saving and estimate lines, and which steps
// the After view marks. Pure; unit-tested in ui/unit/workflowOptimizeView.test.ts.
import type { OptimizeChange, OptimizeDraft, OptimizeEffect, WorkflowGraph } from '../../../lib/api/types';
import { elapsed } from '../../../lib/statusWords.ts';

export const EFFECT_LABEL: Record<OptimizeEffect, string> = {
  same_contract: 'Same result, faster',
  may_change_output: 'Faster; the output may read differently',
  changes_findings: 'May change findings',
};
const ORDER: OptimizeEffect[] = ['same_contract', 'may_change_output', 'changes_findings'];

/** The non-empty groups, in the order the page shows them. */
export function groupChanges(changes: readonly OptimizeChange[]): { effect: OptimizeEffect; label: string; changes: OptimizeChange[] }[] {
  return ORDER.map((effect) => ({ effect, label: EFFECT_LABEL[effect], changes: changes.filter((c) => c.effect === effect) })).filter((g) => g.changes.length);
}

/** What starts ticked: exactly the server's pre-selected changes. */
export function initialSelection(changes: readonly OptimizeChange[]): string[] {
  return changes.filter((c) => c.preselected).map((c) => c.id);
}

export const canApply = (selected: readonly string[]): boolean => selected.length > 0;

/** "−4.7m" / "−40s"; empty when unknown. */
export function savingText(ms?: number | null): string {
  if (!ms || ms <= 0) return '';
  return ms < 60_000 ? `−${Math.round(ms / 1000)}s` : `−${(ms / 60_000).toFixed(1)}m`;
}

/** The estimate card's lines; `none` under 3 runs (no minutes then). */
export function estimateLines(e: OptimizeDraft['estimate']): { typical?: string; slow?: string; basis?: string; none?: string } {
  if (e.runs < 3 || e.typical_before_ms == null || e.typical_after_ms == null) {
    return { none: `Not enough runs yet to estimate time (${e.runs} so far). The changes below are still checked.` };
  }
  const since = e.since ? ` since ${new Date(e.since).toLocaleDateString([], { month: 'short', day: 'numeric' })}` : '';
  return {
    typical: `about ${elapsed(e.typical_before_ms)} → ${elapsed(e.typical_after_ms)} (typical)`,
    ...(e.slow_before_ms != null && e.slow_after_ms != null ? { slow: `slow runs ${elapsed(e.slow_before_ms)} → ${elapsed(e.slow_after_ms)}` } : {}),
    basis: `from ${e.runs} runs${since}`,
  };
}

/** The After view's marks: steps that are new, and steps a ticked change touches. */
export function afterMarks(base: WorkflowGraph, after: WorkflowGraph, changes: readonly OptimizeChange[], selected: readonly string[]): Record<string, string> {
  const had = new Set(base.nodes.map((n) => n.id));
  const marks: Record<string, string> = {};
  for (const c of changes) if (selected.includes(c.id)) for (const id of c.nodes) marks[id] = c.rule === 'parallel' ? 'side by side' : c.rule === 'single_repo_skip' ? 'skipped on one repo' : 'changed';
  for (const n of after.nodes) if (!had.has(n.id)) marks[n.id] = 'new';
  return marks;
}
