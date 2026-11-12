// Vault full-text index v2: a partial index is always said out loud (chip,
// search notice) and the wording never shows NaN or >100 %.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { searchIndexChip, searchIndexDone, searchIndexNotice, searchIndexPercent } from '../src/modules/vault/searchIndex.ts';

test('percent is floored, clamped and complete for an empty vault', () => {
  assert.equal(searchIndexPercent({ state: 'rebuilding', indexed: 4123, total: 10324 }), 39);
  assert.equal(searchIndexPercent({ state: 'rebuilding', indexed: 9999, total: 10000 }), 99, 'never rounds up to done');
  assert.equal(searchIndexPercent({ state: 'rebuilding', indexed: 12, total: 10 }), 100);
  assert.equal(searchIndexPercent({ state: 'pending', indexed: 0, total: 0 }), 100);
  assert.equal(searchIndexChip({ state: 'pending', indexed: 0, total: 7 }), 'Search index 0%');
});

test('the notice states counts and that results may be incomplete', () => {
  const rebuilding = searchIndexNotice({ state: 'rebuilding', indexed: 4123, total: 10324 });
  assert.match(rebuilding, /is rebuilding/);
  assert.match(rebuilding, /4,123 of 10,324 notes indexed/);
  assert.match(rebuilding, /Results may be incomplete\./);
  assert.match(searchIndexNotice({ state: 'pending', indexed: 0, total: 3 }), /waiting to rebuild/);
  assert.equal(searchIndexDone(1), '1 note indexed. Search results are complete again.');
  assert.match(searchIndexDone(10324), /^10,324 notes indexed/);
});
