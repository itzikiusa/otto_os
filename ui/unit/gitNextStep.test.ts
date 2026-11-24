// Git's commit card (lib/gitNextStep.ts): ONE primary that follows the
// checkout's state, and the state in words for the repository header.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { forgeLabel, primaryStep, stateWords, type GitActionState } from '../src/lib/gitNextStep.ts';

const S: GitActionState = { changes: 0, branch: 'feat/x', upstream: 'origin/feat/x', ahead: 0, remote: true, forge: true, prNumber: null };

test('the primary walks Commit and push → Push (↑N) → Create pull request → Open pull request', () => {
  assert.deepEqual(primaryStep({ ...S, changes: 3 }), { kind: 'commit', label: 'Commit and push' });
  assert.deepEqual(primaryStep({ ...S, ahead: 2 }), { kind: 'push', label: 'Push (↑2)' });
  assert.deepEqual(primaryStep(S), { kind: 'create-pr', label: 'Create pull request' });
  assert.deepEqual(primaryStep({ ...S, prNumber: 7 }), { kind: 'pr', label: 'Open pull request' });
});

test('nowhere to push: the primary only commits; unpublished branches publish', () => {
  assert.deepEqual(primaryStep({ ...S, changes: 1, remote: false, upstream: null, forge: false }), { kind: 'commit', label: 'Commit' });
  assert.deepEqual(primaryStep({ ...S, changes: 1, branch: null }), { kind: 'commit', label: 'Commit' });
  assert.deepEqual(primaryStep({ ...S, upstream: null }), { kind: 'publish', label: 'Publish branch' });
  assert.equal(primaryStep({ ...S, remote: false, forge: false, upstream: null }).kind, 'none');
});

test('the header says the state in words', () => {
  const base = { changes: 0, ahead: 0, behind: 0, upstream: 'origin/feat/x', branch: 'feat/x' };
  assert.equal(stateWords(base), 'Clean');
  assert.equal(stateWords({ ...base, changes: 3, ahead: 2 }), '3 files changed · ↑2 not pushed');
  assert.equal(stateWords({ ...base, changes: 1 }), '1 file changed');
  assert.equal(stateWords({ ...base, upstream: null }), 'not published');
  assert.equal(stateWords({ ...base, behind: 4 }), '↓4 to pull');
});

test('forge names: only the forges Otto opens pull requests on', () => {
  assert.equal(forgeLabel({ forge: 'github' } as never), 'GitHub');
  assert.equal(forgeLabel({ forge: 'unrecognized', provider: 'bitbucket' } as never), 'Bitbucket');
  assert.equal(forgeLabel({ forge: null, provider: null } as never), null);
  assert.equal(forgeLabel(null), null);
});
