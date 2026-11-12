// Kafka figures the daemon reports as unknown (`null`) never render as 0:
// the Overview throughput before two sweeps, and a group's list lag.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { lagBadge, nextLagPoll, rateText, totalText } from '../src/modules/brokers/brokerFigures.ts';

test('no throughput samples yet reads "measuring…", not 0', () => {
  assert.equal(rateText(null, false), 'measuring…');
  assert.equal(rateText(undefined, false), 'measuring…');
  assert.equal(totalText(null), '—');
});

test('a failed metrics load reads "—", a measured zero reads 0', () => {
  assert.equal(rateText(null, true), '—');
  assert.equal(rateText(0, false), '0');
  assert.equal(rateText(12.6, false), '13');
  assert.equal(totalText(0), '0');
});

test('group lag: number, pending, unknown and non-consumer groups', () => {
  assert.equal(lagBadge(0, false, 'consumer').text, 'lag 0');
  assert.equal(lagBadge(1500, false, 'consumer').text, `lag ${(1500).toLocaleString()}`);
  assert.equal(lagBadge(null, true, 'consumer').text, 'lag …');
  assert.equal(lagBadge(null, false, 'consumer').text, 'lag —');
  assert.match(lagBadge(null, false, 'connect').title, /Not a consumer group/);
});

test('the list re-polls only while some lag is pending, with a bounded backoff', () => {
  assert.equal(nextLagPoll([{ lag_pending: false }], 0), null);
  assert.equal(nextLagPoll([], 0), null);
  assert.equal(nextLagPoll([{ lag_pending: true }], 0), 5_000);
  assert.equal(nextLagPoll([{ lag_pending: true }], 6), 30_000);
  assert.equal(nextLagPoll([{ lag_pending: true }], 8), null);
});
