import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { listRange } from '../src/modules/git/list-range.ts';

// The Side panel's Files tab (modules/panels/FileTree.svelte) lists a folder's
// entries with no daemon-side cap: opening node_modules or a build output
// folder used to mount every entry (a row button + icons each). It renders a
// ListWindow slice between two spacers instead — pinned here so a rewrite of
// the tree markup can't silently go back to `{#each visibleNodes …}`.

const src = readFileSync(new URL('../src/modules/panels/FileTree.svelte', import.meta.url), 'utf8');

test('the file tree renders a window of its flattened rows, not all of them', () => {
  assert.match(src, /new ListWindow\('\.tree-item'\)/);
  assert.match(src, /\{@attach treeWin\.attach\}/);
  assert.match(src, /\{#each visibleNodes\.slice\(treeRange\.start, treeRange\.end\) as node/);
  assert.doesNotMatch(src, /\{#each visibleNodes as /);
});

test('a 5k-entry folder mounts about a pane of 22 px rows', () => {
  const r = listRange(5_000, 22, 0, 600, 20, 300);
  assert.ok(r.end - r.start < 100, `mounted ${r.end - r.start} rows`);
  assert.equal(r.top + (r.end - r.start) * 22 + r.bottom, 5_000 * 22);
});
