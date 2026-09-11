// Pure sidebar session-order helpers (node:test, Node's built-in type stripping).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { applyOrder, reorder } from '../src/lib/stores/sessionOrder.ts';

const S = (id: string, at: string) => ({ id, last_active_at: at });

test('applyOrder puts unknown ids on top newest-first, then known in order, ignores dead ids', () => {
  const sessions = [
    S('a', '2026-09-01T00:00:00Z'),
    S('b', '2026-09-03T00:00:00Z'),
    S('c', '2026-09-02T00:00:00Z'),
    S('d', '2026-09-05T00:00:00Z'),
    S('e', '2026-09-04T00:00:00Z'),
  ];
  const out = applyOrder(sessions, ['c', 'dead', 'a', 'c']);
  assert.deepEqual(out.map((s) => s.id), ['d', 'e', 'b', 'c', 'a']);
});

test('reorder pulls from and reinserts at to; unknown ids unchanged', () => {
  const ids = ['a', 'b', 'c', 'd'];
  assert.deepEqual(reorder(ids, 'd', 'a'), ['d', 'a', 'b', 'c']);
  assert.deepEqual(reorder(ids, 'a', 'c'), ['b', 'c', 'a', 'd']);
  assert.equal(reorder(ids, 'a', 'a'), ids);
  assert.equal(reorder(ids, 'x', 'a'), ids);
  assert.equal(reorder(ids, 'a', 'x'), ids);
});

test('applyOrder with an empty order equals recency sort', () => {
  const sessions = [
    S('a', '2026-09-01T00:00:00Z'),
    S('b', '2026-09-03T00:00:00Z'),
    S('c', '2026-09-02T00:00:00Z'),
  ];
  assert.deepEqual(applyOrder(sessions, []).map((s) => s.id), ['b', 'c', 'a']);
  assert.deepEqual(applyOrder([], ['a']), []);
});
