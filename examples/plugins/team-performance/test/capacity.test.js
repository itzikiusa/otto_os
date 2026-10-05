// Unit tests for lib/capacity.js. Run: node --test test/capacity.test.js
const { test } = require('node:test');
const assert = require('node:assert/strict');
const C = require('../lib/capacity.js');
const F = require('../lib/flow.js');

const T = (s) => Date.parse(s);
// 2026-06-01 is a Monday; [Jun 1, Jun 27) = 4 full weeks = 20 business days.
const MONTH = { since: T('2026-06-01T00:00:00Z'), until: T('2026-06-27T00:00:00Z') };

test('20-day month with 5 days off → capacity 15', () => {
  const p = { time_off: [{ from: '2026-06-08', to: '2026-06-12' }] };
  assert.deepEqual(C.availability(p, MONTH), { business_days: 20, time_off_days: 5, capacity_days: 15 });
  assert.equal(C.availableDays(p, MONTH), 15);
});

test('per-capacity throughput divides by 15, not 20', () => {
  const people = { a: { time_off: [{ from: '2026-06-08', to: '2026-06-12' }] } };
  const recs = Array.from({ length: 6 }, (_, i) => ({ key: `ABC-${i}`, assignee_id: 'a', done_at: T('2026-06-15T10:00:00Z'), estimate_days: 2 }));
  const m = F.throughputPerWeek(recs, MONTH, { people });
  assert.equal(m.per_person.a.capacity_days, 15);
  assert.equal(m.per_person.a.time_off_days, 5);
  assert.equal(m.per_person.a.count_per_capacity_day, 0.4);
  assert.equal(m.per_person.a.weighted_per_capacity_day, 0.8);
});

test('weekend inside a vacation is not double-subtracted', () => {
  // Fri Jun 5 → Mon Jun 8 = 2 work days, not 4
  assert.equal(C.timeOffDays({ time_off: [{ from: '2026-06-05', to: '2026-06-08' }] }, MONTH), 2);
  // overlapping entries count each day once
  assert.equal(C.timeOffDays({ time_off: [{ from: '2026-06-01', to: '2026-06-03' }, { from: '2026-06-02', to: '2026-06-04' }] }, MONTH), 4);
});

test('holidays and time off outside the window', () => {
  const p = { time_off: [{ from: '2026-05-25', to: '2026-06-01' }, { from: '2026-06-02', to: '2026-06-02' }] };
  // Jun 2 is a holiday → not subtracted twice; only Jun 1 is off
  assert.deepEqual(C.availability(p, MONTH, { holidays: ['2026-06-02'] }), { business_days: 19, time_off_days: 1, capacity_days: 18 });
});

test('teamCapacity sums people', () => {
  const t = C.teamCapacity({ a: { time_off: [{ from: '2026-06-08', to: '2026-06-12' }] }, b: {} }, MONTH);
  assert.equal(t.capacity_days, 35);
  assert.equal(t.time_off_days, 5);
  assert.equal(t.headcount, 2);
});

test('perCapacity: zero denominator → null', () => {
  assert.equal(C.perCapacity(3, 0), null);
  assert.equal(C.perCapacity(3, null), null);
  assert.equal(C.perCapacity(3, 15), 0.2);
  const all = { time_off: [{ from: '2026-06-01', to: '2026-06-30' }] };
  assert.equal(C.availableDays(all, MONTH), 0);
});
