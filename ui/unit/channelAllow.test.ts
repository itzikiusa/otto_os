import { test } from 'node:test';
import assert from 'node:assert/strict';
import { allowedIds, pendingRejected, senderLabel, withAllowed } from '../src/modules/settings/channelAllow.ts';

const at = '2026-10-06T00:00:00Z';

test('the allow-list parses trimmed, blank-free ids', () => {
  assert.deepEqual(allowedIds(''), []);
  assert.deepEqual(allowedIds(' 1, ,2 ,'), ['1', '2']);
});

test('senders already allowed are not offered again', () => {
  const rejected = [
    { user: '111', name: '@alice', at },
    { user: '222', at },
    { user: ' ', at },
  ];
  assert.deepEqual(
    pendingRejected(rejected, '222').map((r) => r.user),
    ['111'],
  );
  assert.deepEqual(pendingRejected(undefined, ''), []);
});

test('allowing a sender appends once, keeping the others', () => {
  assert.equal(withAllowed('', '111'), '111');
  assert.equal(withAllowed('1,2', '111'), '1, 2, 111');
  assert.equal(withAllowed('1, 111', ' 111 '), '1, 111');
});

test('a sender reads as name (id), or the id alone', () => {
  assert.equal(senderLabel({ user: '111', name: '@alice', at }), '@alice (111)');
  assert.equal(senderLabel({ user: '111', name: ' ', at }), '111');
});
