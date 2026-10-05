// Unit tests for lib/flow.js — exact values on a small fixture.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const F = require('../lib/flow.js');

const T = (s) => Date.parse(s);
const W = { since: T('2026-06-01T00:00:00Z'), until: T('2026-06-15T00:00:00Z') }; // 10 business days = 2 weeks
const NOW = T('2026-06-15T00:00:00Z');

const R = [
  { key: 'ABC-1', type: 'Story', epic: 'ABC-100', labels: ['api'], assignee_id: 'a', priority: 'Medium', created_at: T('2026-05-01T00:00:00Z'), started_at: T('2026-06-01T09:00:00Z'), done_at: T('2026-06-03T09:00:00Z'), dev_days: 2, estimate_days: 2, actual_days: 2, phases: { design: 1, dev: 2, review: 0.5, deployment: 1, rework: 0 } },
  { key: 'ABC-2', type: 'Story', epic: 'ABC-100', labels: ['api', 'ui'], assignee_id: 'a', priority: 'Medium', created_at: T('2026-05-01T00:00:00Z'), started_at: T('2026-06-02T09:00:00Z'), done_at: T('2026-06-08T09:00:00Z'), dev_days: 4, estimate_days: 2, actual_days: 4, phases: { design: null, dev: 4, review: 1, deployment: 2, rework: 1 } },
  { key: 'ABC-3', type: 'Bug', assignee_id: 'b', priority: 'High', created_at: T('2026-06-02T00:00:00Z'), started_at: T('2026-06-02T10:00:00Z'), done_at: T('2026-06-04T10:00:00Z'), dev_days: 1, estimate_days: 4, actual_days: 1, phases: { dev: 1, review: 0.5 } },
  { key: 'ABC-4', type: 'Story', assignee_id: 'b', priority: 'Highest', created_at: T('2026-05-01T00:00:00Z'), started_at: T('2026-06-03T09:00:00Z'), done_at: T('2026-06-05T09:00:00Z'), dev_days: 3, estimate_days: 3, actual_days: 3.3 },
  { key: 'ABC-5', type: 'Task', assignee_id: 'b', priority: 'Medium', created_at: T('2026-06-03T00:00:00Z'), started_at: T('2026-06-04T09:00:00Z'), done_at: T('2026-06-10T09:00:00Z'), dev_days: 2, estimate_days: 1, actual_days: 0.4 },
  { key: 'ABC-6', type: 'Story', assignee_id: 'a', scope_excluded: true, started_at: T('2026-06-01T09:00:00Z'), done_at: T('2026-06-02T09:00:00Z'), dev_days: 9, estimate_days: 9, actual_days: 9 },
  { key: 'ABC-7', type: 'Story', assignee_id: 'a', created_at: T('2026-05-01T00:00:00Z'), started_at: T('2026-06-01T09:00:00Z') }, // open, 10 bdays old
  { key: 'ABC-8', type: 'Story', assignee_id: 'b', created_at: T('2026-05-01T00:00:00Z'), started_at: T('2026-06-12T09:00:00Z') }, // open, 1 bday old
];

test('throughputPerWeek: counts and weights, excluding scope_excluded', () => {
  const m = F.throughputPerWeek(R, W);
  assert.deepEqual(m.value, { count: 5, weighted: 12, weeks: 2, count_per_week: 2.5, weighted_per_week: 6 });
  assert.equal(m.n, 5);
  assert.equal(m.coverage, 0.833);
  assert.deepEqual(m.weak_reasons, []);
});

test('wip and agingWip', () => {
  const w = F.wip(R, { now: NOW });
  assert.deepEqual(w.value, { count: 2, by_person: { a: 1, b: 1 }, per_person_mean: 1 });
  const a = F.agingWip(R, { now: NOW });
  // Story cycles (bdays): ABC-1 2, ABC-2 4, ABC-4 2 → p75 = 3
  assert.equal(a.value.p75_by_bucket.Story, 3);
  assert.equal(a.value.count, 1);
  assert.deepEqual(a.value.items[0], { key: 'ABC-7', bucket: 'Story', assignee_id: 'a', age_days: 10, p75_days: 3 });
});

test('contextSwitching: mean distinct keys per person-day', () => {
  const act = [
    { person: 'a', day: '2026-06-01', key: 'ABC-1' }, { person: 'a', day: '2026-06-01', key: 'ABC-2' }, { person: 'a', day: '2026-06-01', key: 'ABC-1' },
    { person: 'a', day: '2026-06-02', key: 'ABC-2' },
    { person: 'b', day: '2026-06-01', key: 'ABC-3' }, { person: 'b', day: '2026-06-01', key: 'ABC-4' }, { person: 'b', day: '2026-06-01', key: 'ABC-5' },
  ];
  const m = F.contextSwitching(act);
  assert.equal(m.value, 2); // (2 + 1 + 3) / 3
  assert.equal(m.n, 3);
  assert.deepEqual(m.by_person, { a: { active_days: 2, mean_keys_per_day: 1.5 }, b: { active_days: 1, mean_keys_per_day: 3 } });
  assert.ok(m.weak_reasons.includes('low_n'));
});

test('investmentMix by type / epic / label', () => {
  const m = F.investmentMix(R, W);
  assert.equal(m.value.total_dev_days, 12);
  assert.deepEqual(m.value.by_type.Story, { days: 9, share: 0.75 });
  assert.deepEqual(m.value.by_type.Bug, { days: 1, share: 0.083 });
  assert.deepEqual(m.value.by_epic['ABC-100'], { days: 6, share: 0.5 });
  assert.deepEqual(m.value.by_label.api, { days: 6, share: 0.5 });
  assert.deepEqual(m.value.by_label['no label'], { days: 6, share: 0.5 });
});

test('unplannedShare: bug, highest priority, interrupt', () => {
  const m = F.unplannedShare(R, W);
  // pool: ABC-1..5, 7, 8 = 7; unplanned: ABC-3 bug, ABC-4 highest, ABC-5 interrupt
  assert.equal(m.n, 7);
  assert.equal(m.value, 0.429);
  assert.deepEqual(m.by_reason, { bug: 1, highest_priority: 1, interrupt: 1 });
  assert.deepEqual(m.keys, ['ABC-3', 'ABC-4', 'ABC-5']);
});

test('estimateAccuracy histogram and ±25%', () => {
  const m = F.estimateAccuracy(R, W);
  // ratios: 1, 2, 0.25, 1.1, 0.4
  assert.deepEqual(m.value.histogram, { '<0.5': 2, '0.5-0.8': 0, '0.8-1.25': 2, '1.25-2': 0, '>2': 1 });
  assert.equal(m.value.pct_within_25, 0.4);
  assert.equal(m.value.median_ratio, 1);
});

test('cycleTimeByPhase: medians, design not tracked is null not 0', () => {
  const m = F.cycleTimeByPhase(R, W);
  assert.equal(m.n, 3);
  assert.deepEqual(m.value.dev, { median: 2, p75: 3, n: 3 });
  assert.deepEqual(m.value.design, { median: 1, p75: 1, n: 1 });
  assert.deepEqual(m.value.review, { median: 0.5, p75: 0.75, n: 3 });
  assert.equal(m.value.total.median, 4.5); // 4.5, 8, 1.5
  assert.ok(m.weak_reasons.includes('design_not_tracked_share'));
  const empty = F.cycleTimeByPhase([{ key: 'ABC-9', done_at: T('2026-06-02T00:00:00Z'), phases: { dev: 1 } }], W);
  assert.equal(empty.value.design.median, null);
  assert.equal(empty.value.design.status, 'not_tracked');
});

test('zero denominators give null', () => {
  const none = { since: T('2026-06-06T00:00:00Z'), until: T('2026-06-08T00:00:00Z') }; // weekend only
  const m = F.throughputPerWeek(R, none, { people: { a: {} } });
  assert.equal(m.value.count_per_week, null);
  assert.equal(m.per_person.a.count_per_capacity_day, null);
  assert.equal(m.coverage, null);
  assert.equal(F.unplannedShare([], W).value, null);
  assert.equal(F.estimateAccuracy([], W).value.pct_within_25, null);
  assert.equal(F.contextSwitching([]).value, null);
});
