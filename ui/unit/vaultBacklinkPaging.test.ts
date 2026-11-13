import { test } from 'node:test';
import assert from 'node:assert/strict';
import { appendBacklinks, backlinksRemaining, toBacklinkPage } from '../src/modules/vault/backlinkPaging.ts';

const row = (path: string, kind = 'wiki') => ({ path, kind, title: path, context: '' });

test('a paged reply passes through; a bare array is every backlink at once', () => {
  const page = { backlinks: [row('a.md')], total: 250, offset: 100, limit: 100 };
  assert.equal(toBacklinkPage(page, 100), page);
  const legacy = toBacklinkPage([row('a.md'), row('b.md')], 100);
  assert.deepEqual([legacy.total, legacy.backlinks.length, legacy.offset], [2, 2, 100]);
});

test('the next page appends without repeating a row the list already shows', () => {
  const shown = [row('a.md'), row('b.md')];
  const next = [row('b.md'), row('b.md', 'embed'), row('c.md')];
  assert.deepEqual(appendBacklinks(shown, next).map((r) => `${r.path}:${r.kind}`), ['a.md:wiki', 'b.md:wiki', 'b.md:embed', 'c.md:wiki']);
});

test('"show more" counts what is not loaded and never goes negative', () => {
  assert.equal(backlinksRemaining(100, 250), 150);
  assert.equal(backlinksRemaining(250, 250), 0);
  assert.equal(backlinksRemaining(260, 250), 0);
});
