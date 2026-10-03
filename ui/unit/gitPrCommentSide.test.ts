import { test } from 'node:test';
import assert from 'node:assert/strict';
import { commentsForLine, indexComments, lineAnchor } from '../src/modules/git/diff-model.ts';
import type { DiffLine, PrComment } from '../src/lib/api/types.ts';

// Inline PR comments key by diff SIDE + line: a deleted row (old 15) and an
// added row (new 15) share a displayed number, and a comment used to render
// under both.

const c = (id: string, line: number | null, side?: 'old' | 'new', outdated = false): PrComment => ({
  id,
  author: 'a',
  body: id,
  path: 'f.ts',
  line,
  created_at: '2026-10-03T00:00:00Z',
  replies: [],
  resolved: false,
  side,
  outdated,
});
const row = (origin: DiffLine['origin'], old_line: number | null, new_line: number | null): DiffLine =>
  ({ origin, old_line, new_line, content: 'x' }) as DiffLine;

test('a deleted-line comment renders under the deleted row only', () => {
  const fc = indexComments([c('del', 15, 'old')]).get('f.ts');
  assert.deepEqual(commentsForLine(fc, row('del', 15, null))?.map((x) => x.id), ['del']);
  assert.equal(commentsForLine(fc, row('add', null, 15)), null);
});

test('a new-side (or side-less) comment renders under the new row only', () => {
  const fc = indexComments([c('new', 15, 'new'), c('legacy', 20)]).get('f.ts');
  assert.deepEqual(commentsForLine(fc, row('add', null, 15))?.map((x) => x.id), ['new']);
  assert.equal(commentsForLine(fc, row('del', 15, null)), null);
  // Context row: matched by its NEW number.
  assert.deepEqual(commentsForLine(fc, row('context', 18, 20))?.map((x) => x.id), ['legacy']);
});

test('outdated and line-less comments go to the file block', () => {
  const fc = indexComments([c('old', 4, 'new', true), c('file', null)]).get('f.ts');
  assert.deepEqual(fc?.unanchored.map((x) => x.id), ['old', 'file']);
  assert.equal(fc?.anchored.size, 0);
});

test('lineAnchor: deleted → old side, everything else → new side', () => {
  assert.deepEqual(lineAnchor(row('del', 7, null)), { side: 'old', line: 7 });
  assert.deepEqual(lineAnchor(row('context', 7, 9)), { side: 'new', line: 9 });
  assert.deepEqual(lineAnchor(row('add', null, 3)), { side: 'new', line: 3 });
});
