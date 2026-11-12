import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  RUN_CAP_SECS, VERIFY_OUTPUT_MARKER, loopStatusOf, pausedByRestart, runCapBinds, splitEvidence,
} from '../src/modules/loops/loopSafety.ts';

const RESTART = 'Interrupted by a daemon restart — Resume to continue with preserved work.';

test('a restart pause is told apart from a person’s pause', () => {
  assert.equal(pausedByRestart({ status: 'paused', error: RESTART }), true);
  assert.equal(pausedByRestart({ status: 'paused', error: null }), false);
  // Boot recovery keeps a blocked loop blocked; its question is the story.
  assert.equal(pausedByRestart({ status: 'blocked', error: RESTART }), false);
  const s = loopStatusOf({ status: 'paused', error: RESTART });
  assert.equal(s.label, 'Paused by restart');
  assert.equal(s.tone, 'warning');
  assert.match(s.hint ?? '', /Resume/);
  assert.equal(loopStatusOf({ status: 'paused', error: null }).label, 'Paused');
  assert.equal(loopStatusOf({ status: 'running', error: null }).label, 'Running');
});

test('the per-run cap binds only when the loop’s own limit leaves more', () => {
  const limits = (max: number) => ({ limits: { max_runtime_secs: max } }) as never;
  assert.equal(RUN_CAP_SECS, 4 * 3600);
  assert.equal(runCapBinds(limits(3600), 0), false);
  assert.equal(runCapBinds(limits(RUN_CAP_SECS + 60), 0), true);
  assert.equal(runCapBinds(limits(RUN_CAP_SECS + 60), 120), false);
});

test('verify output is split from its headline', () => {
  assert.deepEqual(splitEvidence('checked the error message'), { headline: 'checked the error message', output: '' });
  assert.deepEqual(
    splitEvidence(`verify command failed (exit 1)${VERIFY_OUTPUT_MARKER}assertion failed\n  left: 1`),
    { headline: 'verify command failed (exit 1)', output: 'assertion failed\n  left: 1' },
  );
});
