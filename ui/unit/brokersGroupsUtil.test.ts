import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildResetBody, groupStateClass, resetTargetText } from '../src/modules/brokers/groupsUtil.ts';

test('groupStateClass maps Kafka group states to semantic classes', () => {
  assert.equal(groupStateClass('Stable'), 'ok');
  assert.equal(groupStateClass('Empty'), 'dim');
  assert.equal(groupStateClass('Dead'), 'dim');
  assert.equal(groupStateClass('PreparingRebalance'), 'warn');
});

test('buildResetBody builds offset / timestamp / earliest / latest payloads', () => {
  assert.deepEqual(
    buildResetBody({ mode: 'offset', offset: 42, ts: '', topic: 'orders', confirm: true }),
    { mode: 'offset', offset: 42, confirm: true, topic: 'orders' },
  );
  assert.deepEqual(
    buildResetBody({ mode: 'earliest', offset: 0, ts: '', topic: '', confirm: false }),
    { mode: 'earliest', confirm: false },
  );
  const tsBody = buildResetBody({ mode: 'timestamp', offset: 0, ts: '2026-01-02T03:04:05Z', topic: 'events', confirm: true });
  assert.equal(tsBody.mode, 'timestamp');
  assert.equal(tsBody.timestamp_ms, Date.parse('2026-01-02T03:04:05Z'));
  assert.equal(tsBody.topic, 'events');
  assert.equal(tsBody.confirm, true);
});

test('resetTargetText formats human-readable targets', () => {
  assert.equal(resetTargetText('earliest', 0, ''), 'to earliest');
  assert.equal(resetTargetText('offset', 99, ''), 'to offset 99');
  assert.match(resetTargetText('timestamp', 0, '2026-01-02T03:04:05Z'), /^to /);
});
