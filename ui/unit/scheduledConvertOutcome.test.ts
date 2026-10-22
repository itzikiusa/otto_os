import { test } from 'node:test';
import assert from 'node:assert/strict';
import { convertOutcome } from '../src/modules/scheduled-tasks/convertOutcome.ts';

test('a conversion without a trigger id is a failure, never "Converted"', () => {
  for (const trigger_id of [undefined, null, '']) {
    const out = convertOutcome({ workflow_id: 'wf', trigger_id }, 'Nightly', true);
    assert.equal(out.ok, false);
    assert.match(out.body, /without its schedule/);
    assert.doesNotMatch(out.body, /paused the task/);
  }
});

test('a scheduled conversion reports whether the task was paused', () => {
  const paused = convertOutcome({ workflow_id: 'wf', trigger_id: 't1' }, 'Nightly', true);
  assert.deepEqual(paused, { ok: true, title: 'Converted to workflow', body: 'Created a workflow from “Nightly” and paused the task.' });
  const kept = convertOutcome({ workflow_id: 'wf', trigger_id: 't1' }, 'Nightly', false);
  assert.equal(kept.ok, true);
  assert.equal(kept.body, 'Created a workflow from “Nightly”.');
});
