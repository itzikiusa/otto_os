import { test } from 'node:test';
import assert from 'node:assert/strict';
// The read-only goal loop history's status words and evidence split — modules/loops/loopStatus.ts.
import { VERIFY_OUTPUT_MARKER, canContinue, loopStatusOf, pausedByRestart, splitEvidence } from '../src/modules/loops/loopStatus.ts';

const RESTART = 'Interrupted by a daemon restart — Resume to continue with preserved work.';

test('a restart pause is told apart from a person’s pause', () => {
  assert.equal(pausedByRestart({ status: 'paused', error: RESTART }), true);
  assert.equal(pausedByRestart({ status: 'paused', error: null }), false);
  // Boot recovery keeps a blocked loop blocked; its question is the story.
  assert.equal(pausedByRestart({ status: 'blocked', error: RESTART }), false);
  const s = loopStatusOf({ status: 'paused', error: RESTART });
  assert.equal(s.label, 'Paused by restart');
  assert.equal(s.tone, 'warning');
  assert.match(s.hint ?? '', /continue it as a goal/);
  assert.equal(loopStatusOf({ status: 'paused', error: null }).label, 'Paused');
  assert.equal(loopStatusOf({ status: 'running', error: null }).label, 'Running');
});

test('verify output is split from its headline', () => {
  assert.deepEqual(splitEvidence('checked the error message'), { headline: 'checked the error message', output: '' });
  assert.deepEqual(
    splitEvidence(`verify command failed (exit 1)${VERIFY_OUTPUT_MARKER}assertion failed\n  left: 1`),
    { headline: 'verify command failed (exit 1)', output: 'assertion failed\n  left: 1' },
  );
});

test('only an unfinished loop can be continued as a goal', () => {
  for (const s of ['draft', 'paused', 'blocked', 'exhausted']) assert.equal(canContinue({ status: s as never }), true, s);
  for (const s of ['running', 'succeeded', 'failed', 'stopped']) assert.equal(canContinue({ status: s as never }), false, s);
});
