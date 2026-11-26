// "Extend and resume…" proposes caps + 50 %, always above what is used (GL7.2) — workflows/goal/goalExtend.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { extendProblem, extendProposal } from '../src/modules/workflows/goal/goalExtend.ts';

const goal = (iteration: number, max: number, activeMin: number, maxMin?: number) => ({
  iteration, max_iterations: max, criteria_met: 1, criteria_total: 3, active_ms: activeMin * 60_000, ...(maxMin ? { max_active_ms: maxMin * 60_000 } : {}),
});

test('the proposal is the caps + 50 %', () => {
  assert.deepEqual(extendProposal(goal(6, 6, 50, 120)), { iterations: 9, minutes: 180 });
});

test('the proposal is never at or below what is used', () => {
  // Ran past its caps (a stored loop extended earlier): + 1 iteration, + 15 minutes over the use.
  assert.deepEqual(extendProposal(goal(10, 2, 200, 60)), { iterations: 11, minutes: 215 });
});

test('the proposal stays inside the limits a goal accepts', () => {
  assert.deepEqual(extendProposal(goal(40, 40, 400, 400)), { iterations: 50, minutes: 480 });
});

test('totals that leave no room are refused with what is used', () => {
  assert.match(extendProblem(goal(6, 6, 60, 60), 6, 60)!, /6 iterations and 60 minutes/);
  assert.equal(extendProblem(goal(6, 6, 60, 60), 7, 60), null);
  assert.equal(extendProblem(goal(6, 6, 60, 60), 6, 61), null);
});
