// Goal Loops: an evaluated iteration says what the evaluation decided — a run
// that finished but did not meet the goal reads "Continuing", never "Succeeded".
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { iterationStatus } from '../src/modules/loops/loopStatus.ts';
import { runStatus } from '../src/lib/status.ts';

test('a finished iteration that did not meet the goal reads Continuing', () => {
  const s = iterationStatus(runStatus('succeeded'), 'continue');
  assert.equal(s.label, 'Continuing');
  assert.notEqual(s.tone, 'success');
});

test('the goal met and a blocked iteration use their own words', () => {
  assert.equal(iterationStatus(runStatus('succeeded'), 'achieved').label, 'Goal met');
  assert.equal(iterationStatus(runStatus('succeeded'), 'blocked').label, 'Blocked');
});

test('before an evaluation the run status shows', () => {
  assert.equal(iterationStatus(runStatus('running'), undefined).label, runStatus('running').label);
});
