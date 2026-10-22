// accessibility-responsive-4-08: the Side panel's file tree announces its
// hierarchy (tree / treeitem with level, set size and position) and is ONE
// Tab stop (roving tabindex), with "Open folder" off the Tab order.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { componentFunctions } from './componentFunctions.ts';

const url = new URL('../src/modules/panels/FileTree.svelte', import.meta.url);
const src = readFileSync(url, 'utf8');

test('the tree carries tree / treeitem roles with level, size and position', () => {
  assert.match(src, /class="tree-list" role="tree"/);
  assert.match(src, /role="treeitem"/);
  assert.match(src, /aria-level=\{node\.depth \+ 1\}/);
  assert.match(src, /aria-setsize=\{place\?\.size\}/);
  assert.match(src, /aria-posinset=\{place\?\.pos\}/);
  assert.match(src, /tabindex=\{node\.entry\.path === tabRow \? 0 : -1\}/);
  assert.match(src, /class="icon-btn enter-btn" tabindex="-1"/);
});

test('treePositions numbers siblings per parent in a depth-first list', () => {
  const { treePositions } = componentFunctions(url, ['treePositions'], { Map });
  const n = (path: string, depth: number) => ({ depth, entry: { path } });
  const rows = [n('a', 0), n('a/x', 1), n('a/y', 1), n('a/y/q', 2), n('a/z', 1), n('b', 0), n('b/w', 1), n('c', 0)];
  const pos = treePositions(rows) as Map<string, { pos: number; size: number }>;
  const at = (p: string) => { const v = pos.get(p)!; return [v.pos, v.size]; };
  assert.deepEqual(at('a'), [1, 3]);
  assert.deepEqual(at('c'), [3, 3]);
  assert.deepEqual(at('a/x'), [1, 3]);
  assert.deepEqual(at('a/z'), [3, 3]);
  assert.deepEqual(at('a/y/q'), [1, 1]);
  assert.deepEqual(at('b/w'), [1, 1], 'b’s children are their own group, not a’s');
});
