import { test } from 'node:test';
import assert from 'node:assert/strict';
import { preparedNote, setupFailure } from '../src/modules/threads/worktreeReady.ts';

test('a prepared worktree says only what did not copy or clone', () => {
  assert.equal(preparedNote(undefined), null);
  assert.equal(preparedNote({ copied: 2, cloned: ['node_modules'], linked: [], skipped: [] }), null);
  assert.equal(
    preparedNote({ copied: 0, cloned: [], linked: ['node_modules'], skipped: ['big.bin: over the 100 MB copy limit'] }),
    'Linked to the main checkout (this disk can’t clone): node_modules · Left out: big.bin: over the 100 MB copy limit',
  );
});

test('a failed setup names itself, its exit code and its last line', () => {
  assert.equal(setupFailure({ name: 'npm ci', ok: false, exit_code: 1, output: 'added 3\nnpm ERR! missing lockfile\n' }), 'Setup “npm ci” failed (exit 1): npm ERR! missing lockfile');
  assert.equal(setupFailure({ name: 'x', ok: false, exit_code: null, output: '' }), 'Setup “x” failed');
});
