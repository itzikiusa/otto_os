// Unit tests for lib/scope-rules.js — the one delivered-scope predicate.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const SR = require('../lib/scope-rules.js');

const story = (over = {}) => ({ key: 'ABC-1', type: 'Story', assignee_id: 'u-a', ...over });

test('isDeliveredScope: truth table', () => {
  const cases = [
    ['plain story', story(), true],
    ['bug', story({ type: 'Bug' }), true],
    ['rework ticket', story({ rework_of: 'ABC-0' }), false],
    ['scope-excluded rework', story({ scope_excluded: true }), false],
    ['lead-excluded', story({ excluded_override: true }), false],
    ['outlier', story({ outlier: true }), false],
    ['outlier cured by manual time', story({ outlier: true, manual_days: 3 }), true],
    ['stale timing', story({ flags: ['stale_timing'] }), false],
    ['zero time', story({ flags: ['zero_time'] }), false],
    ['epic', story({ type: 'Epic' }), false],
    ['feature type', story({ type: 'Feature' }), false],
    ['feature flag', story({ feature: true }), false],
    ['checklist sub-task', story({ subtask: true, parent_key: 'ABC-2' }), false],
    ['rolled-up sub-task', story({ subtask: true, rollup: true }), false],
    ['substantive but rolled up', story({ subtask: true, substantive_subtask: true, rollup: true }), false],
    ['substantive standalone sub-task', story({ subtask: true, substantive_subtask: true, rollup: false }), true],
    ['story under an epic (parent_key only)', story({ parent_key: 'ABC-9' }), true],
    ['null', null, false],
  ];
  for (const [name, r, want] of cases) assert.equal(SR.isDeliveredScope(r), want, name);
});

test('creditedOwner: substantive sub-task → credited_to; otherwise assignee', () => {
  assert.equal(SR.creditedOwner(story({ subtask: true, substantive_subtask: true, rollup: false, credited_to: 'u-b' })), 'u-b');
  assert.equal(SR.creditedOwner(story({ subtask: true, substantive_subtask: true, rollup: false })), 'u-a');
  assert.equal(SR.creditedOwner(story()), 'u-a');
});

test('weekendOf: complement of the workweek; default Sat/Sun', () => {
  assert.deepEqual(SR.weekendOf([1, 2, 3, 4, 5]), [0, 6]);
  assert.deepEqual(SR.weekendOf([0, 1, 2, 3, 4]), [5, 6]);
  assert.deepEqual(SR.weekendOf(undefined), [0, 6]);
  assert.deepEqual(SR.weekendOf([]), [0, 6]);
});
