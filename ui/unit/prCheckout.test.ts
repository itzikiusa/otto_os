// PR "Check out" (modules/git/pr-checkout.ts): the switch never pulls, so a
// local branch from an earlier checkout can sit behind the PR — the toast must
// say so (with a Pull action) instead of claiming "Checked out".
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { checkoutToast, checkoutVerdict } from '../src/modules/git/pr-checkout.ts';

const st = (upstream: string | null, ahead: number, behind: number) => ({ upstream, ahead, behind });

test('verdicts from the status the checkout returned', () => {
  assert.deepEqual(checkoutVerdict(st('origin/feat', 0, 0)), { kind: 'current' });
  assert.deepEqual(checkoutVerdict(st('origin/feat', 0, 3)), { kind: 'behind', behind: 3, ahead: 0 });
  assert.deepEqual(checkoutVerdict(st('origin/feat', 2, 3)), { kind: 'behind', behind: 3, ahead: 2 });
  assert.deepEqual(checkoutVerdict(st('origin/feat', 2, 0)), { kind: 'ahead', ahead: 2 });
  assert.deepEqual(checkoutVerdict(st(null, 0, 0)), { kind: 'unknown' });
});

test('only an up-to-date branch is a plain success', () => {
  assert.equal(checkoutToast('feat', checkoutVerdict(st('origin/feat', 0, 0))).level, 'success');
  const behind = checkoutToast('feat', checkoutVerdict(st('origin/feat', 0, 1)));
  assert.equal(behind.level, 'warn');
  assert.equal(behind.title, 'feat is behind the pull request');
  assert.match(behind.body, /1 commit behind\. Pull/);
  assert.match(checkoutToast('feat', checkoutVerdict(st('origin/feat', 1, 2))).body, /2 commits behind and 1 commit ahead/);
  assert.equal(checkoutToast('feat', checkoutVerdict(st(null, 0, 0))).level, 'warn');
});
