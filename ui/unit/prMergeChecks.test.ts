// The forge's merge checks + tasks in words (modules/git/pr-merge-checks.ts):
// the side column / phone header line, and the merge dialog's extra reasons
// (CI checks are left to the dialog's own CI row).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mergeCheckReasons, mergeCheckSummary, openTasksLine } from '../src/modules/git/pr-merge-checks.ts';

const ck = (key: string, passed: boolean, label = key) => ({ key, label, passed });

test('summary line counts passed checks', () => {
  assert.equal(mergeCheckSummary([ck('a', true), ck('b', false), ck('c', true)]), '2 of 3 checks passed');
  assert.equal(mergeCheckSummary([ck('a', false)]), '0 of 1 check passed');
  assert.equal(mergeCheckSummary([ck('a', true), ck('b', true)]), 'All 2 checks passed');
  assert.equal(mergeCheckSummary([ck('a', true)]), 'Check passed');
  assert.equal(mergeCheckSummary([]), null);
  assert.equal(mergeCheckSummary(undefined), null);
});

test('open tasks only when some are open', () => {
  assert.equal(openTasksLine({ open: 3, resolved: 1 }), '3 tasks open');
  assert.equal(openTasksLine({ open: 1, resolved: 0 }), '1 task open');
  assert.equal(openTasksLine({ open: 0, resolved: 4 }), null);
  assert.equal(openTasksLine(null), null);
});

test('merge reasons: failing non-CI checks, then open tasks, no duplicates', () => {
  const pr = {
    merge_checks: [ck('minimum_approvals', false, 'At least 2 approvals'), ck('all_builds_passed', false, 'All builds passed'), ck('no_conflicts', true)],
    task_count: { open: 2, resolved: 0 },
  };
  assert.deepEqual(mergeCheckReasons(pr), ['At least 2 approvals', '2 tasks open']);
  // A failing task check already says it: the tasks line is not repeated.
  assert.deepEqual(mergeCheckReasons({ merge_checks: [ck('no_open_tasks', false, 'No open tasks')], task_count: { open: 2, resolved: 0 } }), ['No open tasks']);
  assert.deepEqual(mergeCheckReasons({ merge_checks: [ck('required_status_checks', false, 'Status checks')] }), []);
  assert.deepEqual(mergeCheckReasons({}), []);
});
