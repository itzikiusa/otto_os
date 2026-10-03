import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formatBytes, suggestRetention, RETENTION_PRESETS } from '../src/modules/api/storageGauge.ts';

const gauge = (o: Partial<Record<string, number>> = {}) => ({
  history_rows: 10, history_bytes: 1024, run_rows: 1, step_rows: 5, run_bytes: 100, max_runs_per_automation: 1, ...o,
});
const none = { rows: 0, days: 0, runsKeep: 0 };

test('the storage banner is offered only past a size with no limit for that part', () => {
  assert.equal(suggestRetention(null, none), false);
  assert.equal(suggestRetention(gauge(), none), false);
  assert.equal(suggestRetention(gauge({ history_rows: 5_001 }), none), true);
  assert.equal(suggestRetention(gauge({ history_bytes: 51 * 1024 * 1024 }), none), true);
  assert.equal(suggestRetention(gauge({ history_rows: 9_000 }), { ...none, rows: 1_000 }), false, 'a row limit is set');
  assert.equal(suggestRetention(gauge({ history_rows: 9_000 }), { ...none, days: 30 }), false, 'an age limit is set');
  assert.equal(suggestRetention(gauge({ max_runs_per_automation: 201 }), none), true);
  assert.equal(suggestRetention(gauge({ max_runs_per_automation: 201 }), { ...none, runsKeep: 50 }), false);
});

test('presets always set a limit (none deletes by default) and sizes read', () => {
  for (const p of RETENTION_PRESETS) assert.ok(p.rows > 0 || p.days > 0, p.label);
  assert.equal(formatBytes(512), '512 B');
  assert.equal(formatBytes(2048), '2 KB');
  assert.equal(formatBytes(52 * 1024 * 1024), '52.0 MB');
});
