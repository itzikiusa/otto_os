import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  buildTree, listRows, splitPath, treeRows, type TFolder,
} from '../src/modules/git/wip/wipTree.ts';
import type { FileChange } from '../src/lib/api/types';

// The WIP panel's file-list model: the list view puts the file name first
// with its directory beside it, the tree view folds folders, and both give
// every row the stable key the panel's focus restore and windowing rely on.

const ch = (path: string, kind: FileChange['kind'] = 'modified'): FileChange => ({
  path,
  orig_path: null,
  kind,
  staged: false,
  unstaged: true,
});

test('splitPath separates the file name from its directory', () => {
  assert.deepEqual(splitPath('a/b/c.ts'), { name: 'c.ts', dir: 'a/b' });
  assert.deepEqual(splitPath('README.md'), { name: 'README.md', dir: '' });
});

test('list rows sort by file name, then directory, all at depth 0', () => {
  const rows = listRows([ch('src/z/b.ts'), ch('a.ts'), ch('lib/b.ts')]);
  assert.deepEqual(
    rows.map((r) => r.key),
    ['f:a.ts', 'f:lib/b.ts', 'f:src/z/b.ts'],
  );
  assert.ok(rows.every((r) => r.depth === 0 && r.node.type === 'file'));
  const first = rows[1].node;
  assert.equal(first.type === 'file' && first.dir, 'lib');
});

test('tree rows put folders first and skip a collapsed subtree', () => {
  const tree = buildTree([ch('src/a.ts'), ch('src/deep/b.ts'), ch('top.ts')]);
  const open = treeRows(tree, () => false);
  assert.deepEqual(
    open.map((r) => `${r.depth}:${r.key}`),
    ['0:d:src', '1:d:src/deep', '2:f:src/deep/b.ts', '1:f:src/a.ts', '0:f:top.ts'],
  );
  const shut = treeRows(tree, (n: TFolder) => n.path === 'src');
  assert.deepEqual(shut.map((r) => r.key), ['d:src', 'f:top.ts']);
  // The folder still carries its whole subtree for folder-level actions.
  const src = tree[0];
  assert.equal(src.type === 'folder' && src.files.length, 2);
});

