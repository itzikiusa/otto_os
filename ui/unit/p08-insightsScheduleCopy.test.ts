import { test } from 'node:test';
import assert from 'node:assert/strict';
import { scheduleCopy } from '../src/modules/insights/scheduleStatus.ts';

const off = { daily: false, weekly: false, monthly: false };

test('says off only when the config says off', () => {
  assert.match(scheduleCopy(off).text, /off until you turn them on/);
  assert.equal(scheduleCopy(off).offerTurnOn, true);
  const on = scheduleCopy({ ...off, daily: true });
  assert.doesNotMatch(on.text, /off/);
  assert.equal(on.offerTurnOn, false);
});

test('names a failing cadence with its last error', () => {
  const c = scheduleCopy({
    ...off,
    daily: true,
    schedule_status: {
      daily: { period: 'daily:x', attempts: 3, last_error: 'provider not signed in', gave_up: true, at: '2026-10-23T00:00:00Z' },
    },
  });
  assert.equal(c.failing, true);
  assert.equal(c.text, 'Scheduled daily reports are on, but the last 3 runs didn’t produce a report: provider not signed in');
});

test('a status for a cadence that is now off is ignored', () => {
  const c = scheduleCopy({
    ...off,
    weekly: true,
    schedule_status: { daily: { period: 'd', attempts: 1, gave_up: false, at: '' } },
  });
  assert.equal(c.failing, false);
});
