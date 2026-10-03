import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  ResultBudget,
  MIN_RELEASE_BYTES,
  estimateResultBytes,
  isReleased,
  releasedRows,
  releasedStub,
} from '../src/lib/stores/db-result-budget.ts';

const MB = 1024 * 1024;

test('nothing is released while the total fits the budget', () => {
  const b = new ResultBudget(100 * MB);
  b.note('a', 40 * MB);
  b.note('b', 40 * MB);
  assert.deepEqual(b.pick(new Set()), []);
  assert.equal(b.total, 80 * MB);
});

test('least-recently-viewed results go first, just enough to fit', () => {
  const b = new ResultBudget(100 * MB);
  b.note('a', 40 * MB);
  b.note('b', 40 * MB);
  b.note('c', 40 * MB);
  // Viewing `a` again makes `b` the oldest.
  b.touch('a');
  assert.deepEqual(b.pick(new Set()), ['b']);
  b.note('d', 90 * MB);
  assert.deepEqual(b.pick(new Set()), ['b', 'c', 'a']);
});

test('pinned tabs and small results are never picked', () => {
  const b = new ResultBudget(10 * MB);
  b.note('shown', 50 * MB);
  b.note('tiny', MIN_RELEASE_BYTES - 1);
  b.note('edits', 20 * MB);
  b.note('old', 20 * MB);
  assert.deepEqual(b.pick(new Set(['shown', 'edits'])), ['old']);
  b.forget('old');
  assert.deepEqual(b.pick(new Set(['shown', 'edits'])), []);
  assert.equal(b.has('old'), false);
});

test('size estimate scales with rows, cell payload and batch sets', () => {
  const cols = [{ name: 'id' }, { name: 'name' }];
  const rows = (n: number, s: string) => Array.from({ length: n }, (_, i) => [i, s]);
  const small = estimateResultBytes({ columns: cols, rows: rows(1000, 'x') });
  const big = estimateResultBytes({ columns: cols, rows: rows(100_000, 'x') });
  const fat = estimateResultBytes({ columns: cols, rows: rows(1000, 'x'.repeat(1000)) });
  assert.ok(big > small * 90 && big < small * 110, `${big} vs ${small}`);
  assert.ok(fat > small * 20);
  const docs = estimateResultBytes({ columns: cols, rows: rows(10, '').map((r) => [r[0], { deep: 'y'.repeat(500) }]) });
  assert.ok(docs > 10 * 1000);
  const batch = estimateResultBytes({ columns: cols, rows: rows(1000, 'x'), more_results: [{ columns: cols, rows: rows(1000, 'x') }] });
  assert.equal(batch, small * 2);
  assert.equal(estimateResultBytes(null), 0);
});

test('a released stub keeps columns, drops rows and is recognised by identity', () => {
  const r = { columns: [{ name: 'a' }], rows: [[1], [2], [3]], stats: { duration_ms: 4 } };
  const stub = releasedStub(r);
  assert.equal(stub.rows.length, 0);
  assert.deepEqual(stub.columns, r.columns);
  assert.equal(isReleased(stub), true);
  assert.equal(isReleased(r), false);
  assert.equal(isReleased({ ...stub }), false);
  assert.equal(releasedRows(stub), 3);
  assert.equal(r.rows.length, 3);
});
