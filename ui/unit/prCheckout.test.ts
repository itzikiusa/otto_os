// PR "Check out" (modules/git/pr-checkout.ts): the switch never pulls, so a
// local branch from an earlier checkout can sit behind the PR — the toast must
// say so (with a Pull action) instead of claiming "Checked out".
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { checkoutToast, checkoutVerdict, updateConfirm } from '../src/modules/git/pr-checkout.ts';

const st = (upstream: string | null, ahead: number, behind: number) => ({ upstream, ahead, behind });

test('verdicts from the status the checkout returned', () => {
  assert.deepEqual(checkoutVerdict(st('origin/feat', 0, 0)), { kind: 'current' });
  assert.deepEqual(checkoutVerdict(st('origin/feat', 0, 3)), { kind: 'behind', behind: 3, ahead: 0 });
  // Both sides moved (a force-push since an earlier checkout): no longer
  // "behind" — a fast-forward pull can't fix it (round 2, item 8).
  assert.deepEqual(checkoutVerdict(st('origin/feat', 2, 3)), { kind: 'diverged', behind: 3, ahead: 2 });
  assert.deepEqual(checkoutVerdict(st('origin/feat', 2, 0)), { kind: 'ahead', ahead: 2 });
  assert.deepEqual(checkoutVerdict(st(null, 0, 0)), { kind: 'unknown' });
});

test('only an up-to-date branch is a plain success', () => {
  assert.equal(checkoutToast('feat', checkoutVerdict(st('origin/feat', 0, 0))).level, 'success');
  const behind = checkoutToast('feat', checkoutVerdict(st('origin/feat', 0, 1)));
  assert.equal(behind.level, 'warn');
  assert.equal(behind.title, 'feat is behind the pull request');
  assert.match(behind.body, /1 commit behind\. Pull/);
  const diverged = checkoutToast('feat', checkoutVerdict(st('origin/feat', 1, 2)));
  assert.equal(diverged.level, 'warn');
  assert.equal(diverged.title, 'feat differs from the pull request');
  assert.match(diverged.body, /1 commit the PR doesn’t and is 2 commits behind/);
  assert.match(diverged.body, /backup branch/);
  assert.equal(checkoutToast('feat', checkoutVerdict(st(null, 0, 0))).level, 'warn');
});

test('the update confirm names the move, the backup and the kept changes', () => {
  const c = updateConfirm('feat', { ahead: 1, behind: 2 });
  assert.equal(c.confirmLabel, 'Update branch');
  assert.match(c.title, /Update feat to the pull request\?/);
  assert.match(c.message, /2 commits you don’t have/);
  assert.match(c.message, /Your 1 commit not on the PR stays on a backup branch \(otto-backup\/feat-…\)/);
  assert.match(c.message, /nothing is deleted/);
});
