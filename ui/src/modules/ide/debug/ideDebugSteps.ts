// Pure pieces of the Debug All step line: the chips, the "Step 2 of 4" line,
// the end summary and the single-test Debug of a failed test.
//
// Import-free (type-only imports), so Node runs it directly in unit tests.

import type { IdeDebugStep, IdeDebugStepState } from '../../../lib/api/types/ide';

export interface StepChip {
  index: number;
  /** ✓ passed, ✗ failed, ! could not start, – skipped / no tests, ○ pending, ● running. */
  mark: string;
  text: string;
  state: IdeDebugStepState;
  title: string;
}

const MARKS: Record<IdeDebugStepState, string> = {
  passed: '✓',
  failed: '✗',
  error: '!',
  no_tests: '–',
  skipped: '–',
  pending: '○',
  running: '●',
};

const STATE_WORDS: Record<IdeDebugStepState, string> = {
  passed: 'passed',
  failed: 'failed',
  error: 'couldn’t start',
  no_tests: 'no tests',
  skipped: 'skipped',
  pending: 'waiting',
  running: 'running',
};

/** The step line is for Debug All runs only: one step is an ordinary session. */
export function showSteps(steps: readonly IdeDebugStep[] | null | undefined): boolean {
  return (steps?.length ?? 0) > 1;
}

/** The current step's line: `Step 2 of 4 · Api.Tests (net10.0)`. */
export function currentStepLine(steps: readonly IdeDebugStep[], current: number | null | undefined): string | null {
  if (current == null || !steps[current]) return null;
  return `Step ${current + 1} of ${steps.length} · ${steps[current].label}`;
}

/** One chip per step but the current one (the line names it). */
export function stepChips(steps: readonly IdeDebugStep[], current: number | null | undefined): StepChip[] {
  return steps
    .map((s, index) => {
      const done = !['pending', 'running'].includes(s.state);
      const text = done && s.summary ? `${s.label} ${s.summary}` : s.label;
      return {
        index,
        mark: MARKS[s.state],
        text,
        state: s.state,
        title: `Step ${index + 1} · ${s.label}: ${s.summary || STATE_WORDS[s.state]}`,
      };
    })
    .filter((c) => c.index !== current);
}

/** The end of the run: `4 steps · 3 passed · 1 failed`. */
export function runSummary(steps: readonly IdeDebugStep[]): string {
  const n = (st: IdeDebugStepState): number => steps.filter((s) => s.state === st).length;
  const parts = [`${steps.length} steps`];
  for (const [st, word] of [['passed', 'passed'], ['failed', 'failed'], ['no_tests', 'with no tests'], ['skipped', 'skipped']] as const) {
    if (n(st) > 0) parts.push(`${n(st)} ${word}`);
  }
  const errors = n('error');
  if (errors === 1) parts.push('1 step couldn’t start');
  else if (errors > 1) parts.push(`${errors} steps couldn’t start`);
  return parts.join(' · ');
}

export interface FailedTestRow {
  key: string;
  name: string;
  step: string;
  stepIndex: number;
}

/** Up to 5 failed test names across the run, each tied to its step. */
export function failedTests(steps: readonly IdeDebugStep[], max = 5): FailedTestRow[] {
  const out: FailedTestRow[] = [];
  steps.forEach((s, stepIndex) => {
    for (const name of s.failed_tests ?? []) {
      if (out.length < max) out.push({ key: `${stepIndex}:${name}`, name, step: s.label, stepIndex });
    }
  });
  return out;
}

/** The single-test session a failed test's Debug starts: the step's test
 *  project (or suite source) with that test name and framework. */
export function failedTestLaunch(
  step: IdeDebugStep,
  name: string,
  cwd: string,
): { program: string; args: string[]; cwd: string; test_name: string; is_test: boolean; test_framework?: string } {
  return {
    program: step.program,
    args: [],
    cwd,
    test_name: name,
    is_test: true,
    ...(step.test_framework ? { test_framework: step.test_framework } : {}),
  };
}

/** The console separator a chip scrolls to. */
export function stepSeparatorPrefix(index: number, total: number): string {
  return `── Step ${index + 1}/${total} ·`;
}
