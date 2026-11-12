// Your review on a PR (modules/git/pr-review-state.ts): read from the
// reviewer list, and the optimistic patch applied when Approve / Request
// changes lands — so the header and reviewer list change at once.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { myReview, myReviewer, withMyReview, withoutMyReview } from '../src/modules/git/pr-review-state.ts';
import type { PrDetail, PrReviewer } from '../src/lib/api/types';

const me = (n: string) => n.toLowerCase() === 'itzik';
const rv = (name: string, approved = false, changes_requested = false): PrReviewer => ({ name, approved, changes_requested, avatar_url: null, reviewed_at: null });
const pr = (reviewers: PrReviewer[], approved_by: string[] = []) => ({ reviewers, approved_by }) as unknown as PrDetail;

test('reads your current opinion, changes requested first', () => {
  assert.equal(myReview(pr([rv('Ann', true), rv('Itzik')]), me), null);
  assert.equal(myReview(pr([rv('Itzik', true)]), me), 'approved');
  assert.equal(myReview(pr([rv('Itzik', false, true)]), me), 'changes');
  // GitLab lists approvers only in approved_by on some reads.
  assert.equal(myReview(pr([], ['itzik']), me), 'approved');
});

test('request changes after an approval flips the opinion and drops the approval', () => {
  const next = withMyReview(pr([rv('Ann', true), rv('Itzik', true)], ['Ann', 'Itzik']), 'changes', me, 'itzik');
  assert.deepEqual(next.reviewers.map((r) => [r.name, r.approved, r.changes_requested]), [['Ann', true, false], ['Itzik', false, true]]);
  assert.deepEqual(next.approved_by, ['Ann']);
  assert.equal(myReview(next, me), 'changes');
});

test('approving when not yet a reviewer adds you', () => {
  const next = withMyReview(pr([rv('Ann')]), 'approved', me, 'itzik');
  assert.deepEqual(next.reviewers.map((r) => [r.name, r.approved]), [['Ann', false], ['itzik', true]]);
  assert.deepEqual(next.approved_by, ['itzik']);
  assert.ok(next.reviewers[1].reviewed_at);
  assert.equal(next.reviewers[1].is_me, true);
});

// Bitbucket: the account username is an email, reviewers are display names —
// no handle matches, only the daemon's `is_me` flag says which row is you.
const bbMe = (n: string) => n.toLowerCase() === 'itzik@example.com';
const flagged = (name: string, is_me: boolean, approved = false, changes = false): PrReviewer => ({ ...rv(name, approved, changes), is_me });

test('is_me finds you when your display name matches no handle', () => {
  assert.equal(myReview(pr([flagged('Ann Lee', false, true), flagged('Itzik Cohen', true, true)], ['Ann Lee', 'Itzik Cohen']), bbMe), 'approved');
  assert.equal(myReview(pr([flagged('Itzik Cohen', true, false, true)]), bbMe), 'changes');
  assert.equal(myReview(pr([flagged('Ann Lee', false, true), flagged('Itzik Cohen', true)]), bbMe), null);
});

test('is_me wins over a name match; the handle match is only the fallback', () => {
  // A colleague whose display name happens to equal one of your handles.
  assert.equal(myReviewer([flagged('Itzik', false, true), flagged('Itzik Cohen', true)], me)?.name, 'Itzik Cohen');
  // No reviewer flagged (older daemon, or the identity lookup failed): handles.
  assert.equal(myReviewer([flagged('Itzik', false, true)], me)?.name, 'Itzik');
  assert.equal(myReviewer([rv('Itzik')], me)?.name, 'Itzik');
  assert.equal(myReviewer([flagged('Itzik Cohen', false)], bbMe), undefined);
});

test('approving patches the is_me reviewer in place, never a duplicate', () => {
  const next = withMyReview(pr([flagged('Ann Lee', false), flagged('Itzik Cohen', true)]), 'approved', bbMe, 'itzik@example.com');
  assert.deepEqual(next.reviewers.map((r) => [r.name, r.approved, r.is_me]), [['Ann Lee', false, false], ['Itzik Cohen', true, true]]);
  assert.deepEqual(next.approved_by, ['Itzik Cohen']);
  assert.equal(myReview(next, bbMe), 'approved');
});

test('request changes on Bitbucket drops your display-name approval', () => {
  const next = withMyReview(pr([flagged('Ann Lee', false, true), flagged('Itzik Cohen', true, true)], ['Ann Lee', 'Itzik Cohen']), 'changes', bbMe, 'Itzik');
  assert.deepEqual(next.approved_by, ['Ann Lee']);
  assert.equal(next.reviewers.length, 2);
  assert.equal(myReview(next, bbMe), 'changes');
});

test('the reload after an optimistic approve still reads as approved', () => {
  // Optimistic add (not yet a reviewer, flags present on others), then the
  // server's own list with the real display name and `is_me`.
  const optimistic = withMyReview(pr([flagged('Ann Lee', false)]), 'approved', bbMe, 'Itzik');
  assert.equal(myReview(optimistic, bbMe), 'approved');
  const reloaded = pr([flagged('Ann Lee', false), flagged('Itzik Cohen', true, true)], ['Itzik Cohen']);
  assert.equal(myReview(reloaded, bbMe), 'approved');
});

test('withdrawing an approval keeps you as a reviewer, drops the approval', () => {
  const start = pr([flagged('Ann Lee', false, true), flagged('Itzik Cohen', true, true)], ['Ann Lee', 'Itzik Cohen']);
  const next = withoutMyReview(start, 'approved', bbMe);
  assert.equal(myReview(next, bbMe), null);
  assert.deepEqual(next.approved_by, ['Ann Lee']);
  assert.deepEqual(next.reviewers.map((r) => [r.name, r.approved]), [['Ann Lee', true], ['Itzik Cohen', false]]);
});

test('withdrawing a change request clears it; approve → withdraw → approve round-trips', () => {
  const changes = withMyReview(pr([flagged('Ann Lee', false)]), 'changes', bbMe, 'Itzik');
  assert.equal(myReview(changes, bbMe), 'changes');
  const cleared = withoutMyReview(changes, 'changes', bbMe);
  assert.equal(myReview(cleared, bbMe), null);
  assert.equal(cleared.reviewers.length, 2);
  assert.equal(myReview(withMyReview(cleared, 'approved', bbMe, 'Itzik'), bbMe), 'approved');
  // Withdrawing what isn't yours to withdraw changes nothing.
  const untouched = withoutMyReview(pr([flagged('Ann Lee', false, true)], ['Ann Lee']), 'approved', bbMe);
  assert.deepEqual(untouched.approved_by, ['Ann Lee']);
});
