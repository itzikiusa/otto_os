import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formatUsd, formatCount } from '../src/lib/metric-format.ts';
import { formatCost } from '../src/lib/sessionUsage.ts';

// maintainability-3-07: one cost formatter for every usage / swarm / Mission
// Control / conversation surface (lib/metric-format.ts).

test('formatUsd keeps cents below $1,000 — the same $150.25 in every panel', () => {
  assert.equal(formatUsd(150.25), '$150.25');
  assert.equal(formatUsd(1.239), '$1.24');
  assert.equal(formatUsd(99.999), '$100.00');
  assert.equal(formatUsd(1234.4), '$1,234');
});

test('formatUsd: zero, sub-cent and negative', () => {
  assert.equal(formatUsd(0), '$0');
  assert.equal(formatUsd(0.004), '<$0.01');
  assert.equal(formatUsd(0.0001), '<$0.01');
  assert.equal(formatUsd(-2.5), '−$2.50');
});

test('formatUsd: an unknown cost is "—" by default, or the caller\'s empty', () => {
  assert.equal(formatUsd(null), '—');
  assert.equal(formatUsd(undefined), '—');
  assert.equal(formatUsd(Number.NaN), '—');
  assert.equal(formatUsd(null, { empty: '' }), '');
});

test('the session chip cost agrees with formatUsd', () => {
  for (const n of [0.004, 1.239, 150.25, 1234.4]) assert.equal(formatCost(n), formatUsd(n));
  assert.equal(formatCost(-1), '$0');
});

test('tokens use formatCount (lower-case k)', () => {
  assert.equal(formatCount(12_300), '12.3k');
  assert.equal(formatCount(2_100_000), '2.1M');
});
