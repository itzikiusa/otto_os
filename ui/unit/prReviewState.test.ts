// Your review on a PR (modules/git/pr-review-state.ts): read from the
// reviewer list, and the optimistic patch applied when Approve / Request
// changes lands — so the header and reviewer list change at once.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { myReview, withMyReview } from '../src/modules/git/pr-review-state.ts';
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
});
