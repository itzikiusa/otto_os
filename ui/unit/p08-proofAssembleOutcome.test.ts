import { test } from 'node:test';
import assert from 'node:assert/strict';
import { assembleOutcome } from '../src/modules/proof/assembleOutcome.ts';

test('a folder that is not a repository is not reported as assembled', () => {
  const o = assembleOutcome({ diff: { status: 'not_a_repo' }, commands: [] });
  assert.equal(o.tone, 'info');
  assert.match(o.body, /No git repository at that folder — nothing to assemble/);
});

test('failed git commands are listed', () => {
  const o = assembleOutcome({ diff: { status: 'diff_failed', commands: ['git diff: bad revision'] }, commands: [] });
  assert.equal(o.tone, 'error');
  assert.match(o.body, /git diff: bad revision/);
});

test('an assembled diff succeeds unless a command could not be stored', () => {
  assert.equal(assembleOutcome({ diff: { status: 'assembled', files_changed: 2 }, commands: [] }).tone, 'success');
  const o = assembleOutcome({
    diff: { status: 'assembled', files_changed: 1 },
    commands: [{ cmd: 'cargo test', error: 'disk full' }],
  });
  assert.equal(o.tone, 'error');
  assert.match(o.body, /cargo test.*disk full/);
});
