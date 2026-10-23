import { test } from 'node:test';
import assert from 'node:assert/strict';
import { tabRowIn, treeKeyAction, type TreeKeyRow } from '../src/modules/panels/treeKeys.ts';

// accessibility-responsive-6-06: the side-panel file tree follows the APG tree
// pattern (Hierarchy.svelte's model) and never loses its only Tab stop when
// the active row scrolls out of the window.

const dir = (depth: number, open: boolean): TreeKeyRow => ({ depth, isDir: true, open });
const file = (depth: number): TreeKeyRow => ({ depth, isDir: false, open: false });
//  0 src/ (open)   1 src/lib/ (open)   2 src/lib/a.ts   3 src/b.ts   4 docs/ (closed)   5 README
const rows = [dir(0, true), dir(1, true), file(2), file(1), dir(0, false), file(0)];

test('↑/↓ step, Home/End jump to the ends', () => {
  assert.deepEqual(treeKeyAction('ArrowDown', rows, 2), { kind: 'focus', index: 3 });
  assert.deepEqual(treeKeyAction('ArrowUp', rows, 2), { kind: 'focus', index: 1 });
  assert.deepEqual(treeKeyAction('ArrowUp', rows, 0), { kind: 'none' });
  assert.deepEqual(treeKeyAction('ArrowDown', rows, 5), { kind: 'none' });
  assert.deepEqual(treeKeyAction('Home', rows, 3), { kind: 'focus', index: 0 });
  assert.deepEqual(treeKeyAction('End', rows, 1), { kind: 'focus', index: 5 });
});

test('→ opens a closed folder, then moves into its first child; nothing on a file', () => {
  assert.deepEqual(treeKeyAction('ArrowRight', rows, 4), { kind: 'toggle' });
  assert.deepEqual(treeKeyAction('ArrowRight', rows, 0), { kind: 'focus', index: 1 });
  assert.deepEqual(treeKeyAction('ArrowRight', rows, 2), { kind: 'none' });
  // An open but EMPTY folder has no child to move to.
  assert.deepEqual(treeKeyAction('ArrowRight', [dir(0, true), file(0)], 0), { kind: 'none' });
});

test('← closes an open folder, else moves to the parent', () => {
  assert.deepEqual(treeKeyAction('ArrowLeft', rows, 1), { kind: 'toggle' });
  assert.deepEqual(treeKeyAction('ArrowLeft', rows, 2), { kind: 'focus', index: 1 });
  assert.deepEqual(treeKeyAction('ArrowLeft', rows, 3), { kind: 'focus', index: 0 });
  assert.deepEqual(treeKeyAction('ArrowLeft', rows, 4), { kind: 'none' }, 'a closed top-level folder has no parent');
});

test('RTL mirrors ← and →', () => {
  assert.deepEqual(treeKeyAction('ArrowLeft', rows, 4, true), { kind: 'toggle' });
  assert.deepEqual(treeKeyAction('ArrowRight', rows, 2, true), { kind: 'focus', index: 1 });
});

test('the Tab stop stays on a mounted row', () => {
  const paths = Array.from({ length: 1000 }, (_, i) => `f${i}`);
  assert.equal(tabRowIn(paths, 'f10', 0, 60), 'f10', 'the active row while mounted');
  assert.equal(tabRowIn(paths, 'f10', 400, 460), 'f400', 'scrolled away: the first mounted row');
  assert.equal(tabRowIn(paths, null, 0, 60), 'f0');
  assert.equal(tabRowIn(paths, 'gone', 0, 60), 'f0');
  assert.equal(tabRowIn([], null, 0, 0), null);
});
