import { test } from 'node:test';
import assert from 'node:assert/strict';
import { autoOpenPaths } from '../src/lib/api/jsonTree.ts';

test('a small response opens two levels', () => {
  const open = autoOpenPaths({ a: { x: 1 }, b: [1, 2], c: 3 });
  assert.deepEqual([...open].sort(), ['$', '$.a', '$.b']);
});

test('a root array of wide objects does not open every row (bounded mount)', () => {
  const rows = Array.from({ length: 200 }, (_, i) => Object.fromEntries(Array.from({ length: 30 }, (_, j) => [`f${j}`, i * j])));
  const open = autoOpenPaths(rows);
  const mounted = 200 + [...open].filter((p) => p !== '$').length * 30;
  assert.ok(mounted <= 400, `mounted ${mounted} rows`);
  assert.ok(open.has('$'));
});

test('a scalar root opens nothing but itself', () => {
  assert.deepEqual([...autoOpenPaths(42)], ['$']);
});
