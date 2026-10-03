import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  RUN_HISTORY_CHOICES,
  runHistoryDays,
  runHistoryLabel,
  withRunHistoryDays,
} from '../src/modules/settings/runHistory.ts';

test('run history is off unless a positive window is stored, floored like the daemon', () => {
  assert.equal(runHistoryDays({}), 0);
  assert.equal(runHistoryDays(null), 0);
  assert.equal(runHistoryDays({ data_retention: { enabled: true } }), 0);
  assert.equal(runHistoryDays({ data_retention: { run_history_days: -5 } }), 0);
  assert.equal(runHistoryDays({ data_retention: { run_history_days: '30' } }), 0);
  assert.equal(runHistoryDays({ data_retention: { run_history_days: 3 } }), 14);
  assert.equal(runHistoryDays({ data_retention: { run_history_days: 30 } }), 30);
  assert.equal(RUN_HISTORY_CHOICES[0], 0, 'the default choice keeps forever');
});

test('changing the window keeps every other stored retention field', () => {
  const settings = { data_retention: { enabled: true, audit_log_days: 180, run_history_days: 0 } };
  assert.deepEqual(withRunHistoryDays(settings, 30), {
    enabled: true,
    audit_log_days: 180,
    run_history_days: 30,
  });
  assert.deepEqual(withRunHistoryDays({}, 0), { run_history_days: 0 });
  assert.deepEqual(withRunHistoryDays({ data_retention: [] }, 14), { run_history_days: 14 });
  assert.equal(runHistoryLabel(0), 'Keep forever (default)');
  assert.equal(runHistoryLabel(90), '90 days');
});
