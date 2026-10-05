'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { dayKey, isWorkday, businessDaysTz, distinctDays } = require('../lib/tz');

const U = (s) => Date.parse(s);

test('dayKey: 23:30 local in UTC+3 stays on the local day', () => {
  const t = U('2026-01-05T20:30:00Z'); // 23:30 local in UTC+3
  assert.equal(dayKey(t, 'Europe/Moscow'), '2026-01-05');
  assert.equal(dayKey(U('2026-01-05T21:30:00Z'), 'Europe/Moscow'), '2026-01-06');
  assert.equal(dayKey(U('2026-01-05T21:30:00Z'), 'UTC'), '2026-01-05');
});

test('isWorkday: default Sat/Sun vs configurable Fri/Sat', () => {
  const fri = U('2026-01-09T12:00:00Z');
  const sun = U('2026-01-11T12:00:00Z');
  assert.equal(isWorkday(fri, 'UTC'), true);
  assert.equal(isWorkday(sun, 'UTC'), false);
  assert.equal(isWorkday(fri, 'UTC', [5, 6]), false);
  assert.equal(isWorkday(sun, 'UTC', [5, 6]), true);
});

test('businessDaysTz: weekends, off days and partial days', () => {
  const o = { tz: 'UTC' };
  assert.equal(businessDaysTz(U('2026-01-05T10:00:00Z'), U('2026-01-06T10:00:00Z'), o), 1);
  assert.equal(businessDaysTz(U('2026-01-09T10:00:00Z'), U('2026-01-12T10:00:00Z'), o), 1);
  assert.equal(businessDaysTz(U('2026-01-05T00:00:00Z'), U('2026-01-05T12:00:00Z'), o), 0.5);
  assert.equal(businessDaysTz(U('2026-01-05T00:00:00Z'), U('2026-01-08T00:00:00Z'), { tz: 'UTC', offDays: new Set(['2026-01-06']) }), 2);
  assert.equal(businessDaysTz(U('2026-01-06T00:00:00Z'), U('2026-01-05T00:00:00Z'), o), 0);
  // Local-day boundaries in UTC+3: Sun 21:00Z = Mon 00:00 local.
  assert.equal(businessDaysTz(U('2026-01-04T21:00:00Z'), U('2026-01-05T21:00:00Z'), { tz: 'Europe/Moscow' }), 1);
});

test('distinctDays: local days, deduped and sorted', () => {
  const ts = [U('2026-01-05T20:30:00Z'), U('2026-01-05T08:00:00Z'), U('2026-01-05T21:30:00Z')];
  assert.deepEqual(distinctDays(ts, 'Europe/Moscow'), ['2026-01-05', '2026-01-06']);
  assert.deepEqual(distinctDays(ts, 'UTC'), ['2026-01-05']);
  assert.deepEqual(distinctDays([], 'UTC'), []);
});
