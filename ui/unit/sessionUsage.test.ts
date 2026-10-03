// Per-session tokens + cost labels (lib/sessionUsage.ts, review A5).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formatCost, formatTokens, sessionUsageLabel } from '../src/lib/sessionUsage.ts';

test('token counts read compactly', () => {
  assert.equal(formatTokens(950), '950');
  assert.equal(formatTokens(12_300), '12.3K');
  assert.equal(formatTokens(2_000_000), '2M');
  assert.equal(formatTokens(2_140_000), '2.1M');
  assert.equal(formatTokens(345_000_000), '345M');
  assert.equal(formatTokens(-1), '0');
});

test('cost is an estimate with sensible floors', () => {
  assert.equal(formatCost(0), '$0');
  assert.equal(formatCost(0.004), '<$0.01');
  assert.equal(formatCost(1.239), '$1.24');
  assert.equal(formatCost(1234.4), '$1,234');
});

test('the label puts tokens first and cost second; no usage → null', () => {
  assert.equal(sessionUsageLabel({ total_tokens: 2_140_000, cost_usd: 1.24 }), '2.1M tokens · $1.24');
  assert.equal(sessionUsageLabel({ total_tokens: 0, cost_usd: 0 }), null);
  assert.equal(sessionUsageLabel(null), null);
});
