import { test } from 'node:test';
import assert from 'node:assert/strict';
import { budgetHeadsUp, costRows, localDay, monthOutlook } from '../src/modules/usage/usageOutlook.ts';

const days = (from: string, n: number, cost: number) => {
  const start = new Date(`${from}T12:00:00`);
  return Array.from({ length: n }, (_, i) => {
    const d = new Date(start);
    d.setDate(start.getDate() + i);
    return { day: localDay(d), cost_usd: cost };
  });
};

test('month to date counts only this month, and projects the rest at the recent pace', () => {
  // Oct 20 → Nov 10 at $10/day; today is Nov 10 (30 days in November).
  const now = new Date('2026-11-10T15:00:00');
  const o = monthOutlook(days('2026-10-20', 22, 10), 30, now);
  assert.ok(o);
  assert.equal(o.monthToDate, 100);
  assert.equal(o.paceUsd, 10);
  assert.equal(o.projected, 100 + 10 * 20);
});

test('a window that does not reach the 1st says nothing about the month', () => {
  const now = new Date('2026-11-20T09:00:00');
  assert.equal(monthOutlook(days('2026-11-14', 7, 5), 7, now), null);
  assert.ok(monthOutlook(days('2026-11-14', 7, 5), 30, now));
});

test('the pace is the last 7 calendar days, not the last 7 rows', () => {
  // Spend stopped two weeks ago: nothing this month, no pace — no outlook.
  assert.equal(monthOutlook(days('2026-10-01', 20, 9), 60, new Date('2026-11-10T10:00:00')), null);
  // Two days of $14 in the last week → $4/day.
  const o = monthOutlook([{ day: '2026-11-09', cost_usd: 14 }, { day: '2026-11-10', cost_usd: 14 }], 30, new Date('2026-11-10T10:00:00'));
  assert.ok(o);
  assert.equal(o.paceUsd, 4);
  assert.equal(o.projected, 28 + 4 * 20);
});

test('no spend at all is no outlook, not "$0 on track for $0"', () => {
  assert.equal(monthOutlook([], 30, new Date('2026-11-10T10:00:00')), null);
  assert.equal(monthOutlook(days('2026-11-01', 10, 0), 30, new Date('2026-11-10T10:00:00')), null);
});

test('budgets reach the first screen only at 80 % or more, fullest first', () => {
  const rows = [
    { scope: 'provider', key: 'codex', label: 'Codex', used_fraction: 0.5, exceeded: false },
    { scope: 'provider', key: 'claude', label: 'Claude', used_fraction: 0.92, exceeded: false },
    { scope: 'workspace', key: 'w1', label: null, used_fraction: 1.3, exceeded: true },
  ];
  const lines = budgetHeadsUp(rows, (r) => r.label ?? 'Harbor');
  assert.deepEqual(lines, [
    { text: 'Harbor budget is used up', over: true },
    { text: 'Claude budget 92 % used', over: false },
  ]);
});

test('cost rows put the biggest cost first and size bars against it', () => {
  const t = (cost_usd: number, total_tokens: number) => ({ input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, total_tokens, cost_usd });
  const rows = costRows([{ ...t(5, 900), id: 'a' }, { ...t(20, 100), id: 'b' }, { ...t(5, 1000), id: 'c' }], (x) => x.id, (x) => x.id.toUpperCase());
  assert.deepEqual(rows.map((r) => [r.key, r.label, r.share]), [['b', 'B', 100], ['c', 'C', 25], ['a', 'A', 25]]);
});
