// Goal-loop budget helpers (L1): which hard cap stopped a loop, whether a
// plain Resume would get anywhere, and the "Extend budget…" prefill.
import type { GoalLoop } from '../../lib/api/types';

export type BudgetCap = 'iterations' | 'runtime';

/** The cap the loop has hit: the engine's `error` names it ("iteration cap
 *  reached" / "time cap reached"); fall back to the counters. */
export function budgetCap(loop: GoalLoop, elapsedSecs: number): BudgetCap | null {
  const err = loop.error ?? '';
  if (/iteration cap/i.test(err)) return 'iterations';
  if (/time cap/i.test(err)) return 'runtime';
  if (loop.iterations_started >= loop.limits.max_iterations) return 'iterations';
  if (elapsedSecs >= loop.limits.max_runtime_secs) return 'runtime';
  return null;
}

/** True when the remaining budget lets a plain Resume start an iteration. */
export function hasBudgetLeft(loop: GoalLoop, elapsedSecs: number): boolean {
  return (
    loop.iterations_started < loop.limits.max_iterations && elapsedSecs < loop.limits.max_runtime_secs
  );
}

/** Prefill: the current caps + 50% (at least +1 iteration / +1 minute), and
 *  never below what is already used — so the default always gives room. */
export function extendDefaults(
  loop: GoalLoop,
  elapsedSecs: number,
): { max_iterations: number; runtime_minutes: number } {
  const it = loop.limits.max_iterations;
  const mins = Math.ceil(loop.limits.max_runtime_secs / 60);
  const usedMins = Math.ceil(elapsedSecs / 60);
  return {
    max_iterations: Math.max(it + Math.max(1, Math.ceil(it * 0.5)), loop.iterations_started + 1),
    runtime_minutes: Math.max(mins + Math.max(1, Math.ceil(mins * 0.5)), usedMins + 1),
  };
}
