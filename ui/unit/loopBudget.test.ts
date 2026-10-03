import { test } from 'node:test';
import assert from 'node:assert/strict';
import { budgetCap, extendDefaults, hasBudgetLeft } from '../src/modules/loops/budget.ts';

function loop(over: Record<string, unknown> = {}) {
  return {
    iterations_started: 5,
    error: null,
    limits: { max_iterations: 5, max_runtime_secs: 3600, per_phase_timeout_secs: 600, max_attempts_per_executor: 2 },
    ...over,
  } as never;
}

test('the cap named by the engine error wins, else the counters decide', () => {
  assert.equal(budgetCap(loop({ error: 'time cap reached' }), 10), 'runtime');
  assert.equal(budgetCap(loop({ error: 'iteration cap reached' }), 10), 'iterations');
  assert.equal(budgetCap(loop(), 10), 'iterations');
  assert.equal(budgetCap(loop({ iterations_started: 1 }), 3600), 'runtime');
  assert.equal(budgetCap(loop({ iterations_started: 1 }), 10), null);
});

test('a plain resume is offered only while budget remains', () => {
  assert.equal(hasBudgetLeft(loop(), 10), false);
  assert.equal(hasBudgetLeft(loop({ iterations_started: 4 }), 10), true);
  assert.equal(hasBudgetLeft(loop({ iterations_started: 4 }), 3600), false);
});

test('the extend prefill is current + 50% and always above what is used', () => {
  assert.deepEqual(extendDefaults(loop(), 3600), { max_iterations: 8, runtime_minutes: 90 });
  // Already past the cap (e.g. a long phase overran): stays above usage.
  assert.deepEqual(extendDefaults(loop({ iterations_started: 9 }), 6000), { max_iterations: 10, runtime_minutes: 101 });
  // Tiny caps still grow by at least one.
  const tiny = loop({ iterations_started: 1, limits: { max_iterations: 1, max_runtime_secs: 60, per_phase_timeout_secs: 60, max_attempts_per_executor: 1 } });
  assert.deepEqual(extendDefaults(tiny, 60), { max_iterations: 2, runtime_minutes: 2 });
});
