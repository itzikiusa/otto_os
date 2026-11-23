// The thread details card (threadDetails.ts): one git action that follows the
// checkout's state, and where the card may float.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { cardFloats, gitAction, type GitActionState } from '../src/modules/threads/threadDetails.ts';

const S: GitActionState = { changes: 0, branch: 'feat/x', upstream: 'origin/feat/x', ahead: 0, remote: true, forge: true, prNumber: null };

test('the git action walks Commit → Push → Create PR → PR #N', () => {
  assert.deepEqual(gitAction({ ...S, changes: 3 }), { kind: 'commit', label: 'Commit' });
  assert.deepEqual(gitAction({ ...S, upstream: null }), { kind: 'publish', label: 'Publish branch' });
  assert.deepEqual(gitAction({ ...S, ahead: 1 }), { kind: 'push', label: 'Push 1 commit' });
  assert.equal(gitAction({ ...S, ahead: 4 }).label, 'Push 4 commits');
  assert.deepEqual(gitAction(S), { kind: 'create-pr', label: 'Create PR' });
  assert.deepEqual(gitAction({ ...S, prNumber: 12 }), { kind: 'pr', label: 'PR #12' });
});

test('new local work comes before the open pull request; nothing to do says so', () => {
  assert.equal(gitAction({ ...S, prNumber: 12, changes: 1 }).kind, 'commit');
  assert.equal(gitAction({ ...S, prNumber: 12, ahead: 2 }).kind, 'push');
  // No remote: nothing to push or open a PR on.
  assert.equal(gitAction({ ...S, remote: false, forge: false, upstream: null }).kind, 'none');
  // Detached HEAD: no branch to publish.
  assert.equal(gitAction({ ...S, branch: null, upstream: null }).kind, 'none');
});

test('the card floats only beside one conversation with a full reading column left', () => {
  const base = { centerWidth: 1284, compact: false, rightPanel: false, panes: 1, altView: false };
  assert.equal(cardFloats(base), true);
  assert.equal(cardFloats({ ...base, centerWidth: 1124 }), false);
  assert.equal(cardFloats({ ...base, rightPanel: true }), false);
  assert.equal(cardFloats({ ...base, compact: true }), false);
  assert.equal(cardFloats({ ...base, panes: 2 }), false);
  assert.equal(cardFloats({ ...base, altView: true }), false);
});
