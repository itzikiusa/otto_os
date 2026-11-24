// "Open diff" on a turn's changes card finds the file in Changes
// (modules/threads/changesFocus.ts) whichever side carries the subfolder.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { focusRow, focusSettled } from '../src/modules/threads/changesFocus.ts';

const rows = [{ path: 'README.md' }, { path: 'ui/src/a.ts' }];

test('a thread in a subfolder of the repo: the card path is the shorter one', () => {
  assert.equal(focusRow(rows, 'src/a.ts')?.path, 'ui/src/a.ts');
});

test('an absolute or longer card path still finds the repo row; no match is null', () => {
  assert.equal(focusRow(rows, '/repo/ui/src/a.ts')?.path, 'ui/src/a.ts');
  assert.equal(focusRow(rows, 'README.md')?.path, 'README.md');
  assert.equal(focusRow(rows, 'src/b.ts'), null);
  assert.equal(focusRow(rows, null), null);
});

test('the ask is dropped only once THIS repo\'s list has loaded without the file', () => {
  // Rows left over from another repo, or still loading: kept.
  assert.equal(focusSettled({ hit: false, loading: false, loadedFor: 'repo-old', repoId: 'repo-new' }), false);
  assert.equal(focusSettled({ hit: false, loading: true, loadedFor: 'repo-new', repoId: 'repo-new' }), false);
  assert.equal(focusSettled({ hit: false, loading: false, loadedFor: null, repoId: 'repo-new' }), false);
  // This repo's list is there: shown, or not in it — done either way.
  assert.equal(focusSettled({ hit: false, loading: false, loadedFor: 'repo-new', repoId: 'repo-new' }), true);
  assert.equal(focusSettled({ hit: true, loading: true, loadedFor: null, repoId: 'repo-new' }), true);
});
