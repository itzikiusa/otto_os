'use strict';
const test = require('node:test');
const assert = require('node:assert');
const M = require('../lib/metrics.js');

const DAY = 86400000;
const t0 = Date.UTC(2026, 0, 5, 9); // Monday

function scopeOf(records, people = {}) {
  return {
    records, estimates: { 'ABC-1': { days: 2 }, 'ABC-2': { days: 1 } }, people, flat_people: { u1: { name: 'Person One' } },
    canonical: (id) => id, config: { workweek: [1, 2, 3, 4, 5], qa_work_min_commit_days: 2, timezone: 'UTC' },
  };
}

test('weekendOf converts a workweek into weekend days', () => {
  assert.deepStrictEqual(M.weekendOf([1, 2, 3, 4, 5]), [0, 6]);
  assert.deepStrictEqual(M.weekendOf([0, 1, 2, 3, 4]), [5, 6]);
});

test('offDaySet expands inclusive time-off ranges', () => {
  const s = M.offDaySet({ time_off: [{ from: '2026-01-05', to: '2026-01-07' }] });
  assert.deepStrictEqual([...s], ['2026-01-05', '2026-01-06', '2026-01-07']);
});

test('computeMetrics: capacity uses time off, design not tracked stays null, rework excluded from scope', () => {
  const recs = [
    { key: 'ABC-1', type: 'Story', assignee_id: 'u1', status_category: 'done', status: 'Done', created: t0 - DAY, done_at: t0 + 3 * DAY, eff_done_at: t0 + 3 * DAY, first_active_at: t0, intervals: [{ status: 'In Progress', from: t0, to: t0 + 2 * DAY }], commit_ts: [t0 + DAY] },
    { key: 'ABC-2', type: 'Bug', assignee_id: 'u1', status_category: 'done', status: 'Done', created: t0 + 4 * DAY, done_at: t0 + 6 * DAY, eff_done_at: t0 + 6 * DAY, first_active_at: t0 + 4 * DAY, intervals: [], scope_excluded: true, rework_of: 'ABC-1' },
  ];
  const withPh = M.attachPhases(recs, { prMap: new Map(), tags: [], config: scopeOf([]).config, people: {}, canonical: (x) => x });
  assert.strictEqual(withPh[0].phases.design.days, null, 'no design evidence → not tracked');
  const scope = scopeOf(withPh, { u1: { time_off: [{ from: '2026-01-12', to: '2026-01-16' }] } });
  const m = M.computeMetrics(scope, { since: t0, until: t0 + 14 * DAY }, { tags: [], prs: null });
  assert.strictEqual(m.capacity.people.u1.time_off_days, 5);
  assert.strictEqual(m.capacity.people.u1.capacity_days, 5);
  assert.ok(m.guardrails.some((g) => g.code === 'no_deploy_tags'));
  assert.strictEqual(m.flow.throughputPerWeek.value.count, 1, 'rework ticket is not delivered scope');
});
