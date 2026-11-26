// The step pane's ⋯ menu (StepPane.svelte `stepMenu`): only a failed step
// re-runs alone, and only a step that ran re-runs with what follows — the
// server refuses a pending or running entry step (routes/workflows.rs
// retry-node), so the menu never offers one (CR1-044).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';

function menu(runStatus: string, nodeStatus: string): string[] {
  let shown: { label?: string }[] = [];
  const pane = componentFunctions(new URL('../src/modules/workflows/runs/StepPane.svelte', import.meta.url), ['stepMenu'], {
    run: { id: 'r1', status: runStatus },
    node: { node_id: 'n1', status: nodeStatus },
    step: { kind: 'agent_prompt' },
    ctxMenu: { showAt: (_el: unknown, items: { label?: string }[]) => { shown = items; } },
  });
  pane.stepMenu({ currentTarget: {} });
  // Built here, not in the component's realm, so deepEqual compares plain arrays.
  const labels: string[] = [];
  for (const i of shown) if (i.label?.startsWith('Retry')) labels.push(i.label);
  return labels;
}

test('a settled run offers the retry each step can take', () => {
  assert.deepEqual(menu('error', 'error'), ['Retry only this step']);
  assert.deepEqual(menu('error', 'success'), ['Retry from here']);
  assert.deepEqual(menu('canceled', 'skipped'), ['Retry from here']);
  assert.deepEqual(menu('error', 'pending'), [], 'a step the run never reached has nothing to retry');
  assert.deepEqual(menu('canceled', 'running'), []);
});

test('a live run offers no retry', () => {
  assert.deepEqual(menu('running', 'error'), []);
});
