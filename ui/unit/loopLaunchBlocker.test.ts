// A8: the disabled Launch says what is actually missing, first thing first.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { launchBlocker, type LaunchForm } from '../src/modules/loops/launchBlocker.ts';

const ok: LaunchForm = {
  name: 'Angular feature', seed: 'Add a hello function', mode: 'build', repoPath: '/r', graphIssue: null, graphTasks: 1,
  maxIterations: 5, maxMinutes: 60, perPhaseMinutes: 20, executorCount: 1,
  criteria: [{ text: 'builds', verify: 'npm run build', verify_kind: 'command', verify_cmd: 'npm run build' }],
};

test('a complete form launches', () => {
  assert.equal(launchBlocker(ok), null);
});

test('the reason names the first missing thing, not the name and criteria', () => {
  assert.equal(launchBlocker({ ...ok, seed: '', repoPath: '' }), 'Describe the goal.');
  assert.equal(launchBlocker({ ...ok, repoPath: ' ' }), 'Pick the repository to work in.');
  assert.equal(launchBlocker({ ...ok, mode: 'research', repoPath: '' }), null);
  assert.equal(launchBlocker({ ...ok, criteria: [] }), 'Add at least one acceptance criterion.');
  assert.equal(launchBlocker({ ...ok, criteria: [{ ...ok.criteria[0], verify_cmd: '' }] }), 'A command criterion needs its shell command.');
  assert.equal(launchBlocker({ ...ok, executorCount: 7 }), 'Budget values must be whole numbers above 0, with at most 6 executors.');
  assert.equal(launchBlocker({ ...ok, graphTasks: 9 }), 'Increase the iteration budget to cover every graph task.');
  assert.equal(launchBlocker({ ...ok, graphIssue: 'cycle' }), 'Fix the execution graph.');
});
