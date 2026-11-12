// Round 2 of the PR fixes: the files follow a push (pr-head-follow.ts),
// "Changes since your review" (pr-since-review.ts), adding / removing
// reviewers (pr-reviewers-edit.ts), the "review requested from you" list
// query (pr-list-prefs.ts) and the optimistic comment insert (pr-comments.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { headMoved, headMovedLabel, newCommitCount } from '../src/modules/git/pr-head-follow.ts';
import {
  isRangePick,
  pickLabel,
  pickTarget,
  rememberReviewedHead,
  reviewedHead,
  sameSha,
  sinceReviewItems,
  sinceReviewPick,
} from '../src/modules/git/pr-since-review.ts';
import { currentReviewers, reviewerEdit, reviewerEditWhat, reviewerEditWho } from '../src/modules/git/pr-reviewers-edit.ts';
import { withComment } from '../src/modules/git/pr-comments.ts';
import { loadSource } from './sourceHarness.ts';
import type { PrComment, PrCommit, PrDetail } from '../src/lib/api/types';

const sha = (s: string) => ({ sha: s });

test('the files follow a moved head; an unknown head never counts as moved', () => {
  assert.equal(headMoved('aaa1111', 'bbb2222'), true);
  assert.equal(headMoved('aaa1111', 'aaa1111'), false);
  assert.equal(headMoved('', 'bbb2222'), false, 'nothing shown yet');
  assert.equal(headMoved('aaa1111', null), false, 'the forge said nothing');
});

test('new commits are counted against the commits on screen', () => {
  assert.equal(newCommitCount([sha('a'), sha('b')], [sha('c'), sha('a'), sha('b')]), 1);
  // A force-push rewrote both: every commit is new to the reader.
  assert.equal(newCommitCount([sha('a'), sha('b')], [sha('a2'), sha('b2')]), 2);
  assert.equal(newCommitCount(null, [sha('a')]), null);
  assert.equal(newCommitCount([sha('a')], null), null);
  assert.equal(headMovedLabel(1), '1 new commit · files updated');
  assert.equal(headMovedLabel(3), '3 new commits · files updated');
  assert.equal(headMovedLabel(null), 'New commits · files updated');
  assert.equal(headMovedLabel(0), 'New commits · files updated');
});

test('"since your review" is offered only when the head moved after it', () => {
  assert.equal(sinceReviewPick(null, 'bbb2222bbb'), null, 'never reviewed here');
  assert.equal(sinceReviewPick('aaa1111aaa', null), null);
  assert.equal(sinceReviewPick('aaa1111aaa', 'aaa1111aaa'), null);
  // Bitbucket abbreviates: the same commit, so nothing new.
  assert.equal(sinceReviewPick('aaa1111aaa0000', 'aaa1111aaa00'), null);
  assert.equal(sinceReviewPick('aaa1111aaa', 'bbb2222bbb'), 'range:aaa1111aaa..bbb2222bbb');
  assert.equal(sameSha('abc', 'abc'), false, 'too short to trust');
});

test('a picker selection is a commit or a range, with its target and label', () => {
  const pick = 'range:aaa1111..bbb2222';
  assert.equal(isRangePick(pick), true);
  assert.equal(pickTarget(pick), pick);
  assert.equal(pickTarget('c1fac680'), 'commit:c1fac680');
  const commit = { sha: 'c1fac680', subject: 'feat: vip' } as PrCommit;
  assert.equal(pickLabel(null, null), 'All commits');
  assert.equal(pickLabel(pick, null), 'Changes since your review');
  assert.equal(pickLabel('c1fac680', commit), 'feat: vip');
  assert.equal(pickLabel('c1fac680aa', null), 'c1fac68');
  let chosen = '';
  const items = sinceReviewItems(pick, null, (s) => (chosen = s));
  assert.equal(items.length, 1);
  assert.equal(items[0].hint, 'aaa1111');
  assert.equal(items[0].checked, false);
  items[0].action?.();
  assert.equal(chosen, pick);
  assert.equal(sinceReviewItems(pick, pick, () => {})[0].checked, true);
  assert.deepEqual(sinceReviewItems(null, null, () => {}), []);
});

test('the reviewed head is remembered per PR, and storage failures are quiet', () => {
  const store = new Map<string, string>();
  const g = globalThis as { localStorage?: unknown };
  const prev = g.localStorage;
  g.localStorage = { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) };
  try {
    assert.equal(reviewedHead('rep', 51), null);
    rememberReviewedHead('rep', 51, '  ');
    assert.equal(reviewedHead('rep', 51), null, 'an empty head is not a review');
    rememberReviewedHead('rep', 51, 'c1fac680');
    assert.equal(reviewedHead('rep', 51), 'c1fac680');
    assert.equal(reviewedHead('rep', 52), null);
    g.localStorage = { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('blocked'); } };
    assert.doesNotThrow(() => rememberReviewedHead('rep', 51, 'x'));
    assert.equal(reviewedHead('rep', 51), null);
  } finally {
    g.localStorage = prev;
  }
});

test('a reviewer edit sends only real changes and the confirm names everyone', () => {
  const current = ['ann', 'Bob'];
  assert.equal(reviewerEdit(current, [], []), null);
  assert.equal(reviewerEdit(current, ['ANN', ' '], []), null, 'already a reviewer');
  assert.deepEqual(reviewerEdit(current, ['@carl', 'carl'], ['bob']), { add: ['carl'], remove: ['Bob'] });
  assert.deepEqual(reviewerEdit(current, [], ['ann']), { remove: ['ann'] });
  const edit = { add: ['carl', 'dee'], remove: ['Bob'] };
  assert.equal(reviewerEditWhat(edit), 'Add: carl, dee\nRemove: Bob');
  assert.match(reviewerEditWho(edit, 'GitHub'), /^carl, dee are asked for a review and notified by GitHub\./);
  assert.match(reviewerEditWho({ add: ['carl'] }, 'GitHub'), /^carl is asked/);
  assert.equal(reviewerEditWho({ remove: ['Bob'] }, 'GitHub'), 'The author and the PR’s reviewers see the change.');
  const pr = (reviewers: string[], approved_by: string[]) =>
    ({ reviewers: reviewers.map((name) => ({ name, approved: false })), approved_by }) as unknown as PrDetail;
  assert.deepEqual(currentReviewers(pr(['ann'], ['zed'])), ['ann']);
  assert.deepEqual(currentReviewers(pr([], ['zed'])), ['zed']);
});

test('a posted reply lands under its thread; anything else goes last', () => {
  const cm = (id: string, extra: Partial<PrComment> = {}): PrComment =>
    ({ id, author: 'a', body: id, path: null, line: null, created_at: '2026-11-12T10:00:00Z', replies: [], ...extra }) as PrComment;
  const list = [cm('1', { replies: [cm('2')] }), cm('3', { thread_id: 't3' })];
  assert.deepEqual(withComment(list, cm('9'), '2')[0].replies.map((r) => r.id), ['2', '9']);
  assert.deepEqual(withComment(list, cm('9'), 't3')[1].replies.map((r) => r.id), ['9']);
  assert.deepEqual(withComment(list, cm('9'), 'nope').map((c) => c.id), ['1', '3', '9']);
  assert.deepEqual(withComment(list, cm('9')).map((c) => c.id), ['1', '3', '9']);
  assert.equal(list[0].replies.length, 1, 'the input is not mutated');
});

test('"review requested from you" is its own list: open only, review=me', () => {
  const prefs = loadSource(new URL('../src/modules/git/pr-list-prefs.ts', import.meta.url), {
    './pr-query': { SORT_LABEL: {} },
  });
  assert.equal(prefs.listToken('merged', false), 'merged');
  assert.equal(prefs.listToken('merged', true), 'merged~review');
  assert.equal(prefs.prListUrl('rep', 'all', 2, 50), '/repos/rep/prs?state=all&page=2&per_page=50');
  assert.equal(prefs.prListUrl('rep', 'merged~review', 1, 50), '/repos/rep/prs?state=open&review=me&page=1&per_page=50');
  assert.equal(prefs.DEFAULT_PREFS.review, false);
  assert.equal(prefs.activeFilters({ ...prefs.DEFAULT_PREFS, review: true }), 1);
  assert.equal(prefs.CLEARED_FILTERS.review, false);
  const set: unknown[] = [];
  const items = prefs.filterItems(prefs.DEFAULT_PREFS, { hasHandles: false, authors: [], pickAuthor: () => {} }, (x: unknown) => set.push(x));
  const item = items.find((i: { label?: string }) => i.label === 'Review requested from you');
  assert.ok(item, 'the Filters menu offers it');
  item.action();
  assert.deepEqual(JSON.parse(JSON.stringify(set)), [{ review: true }]);
});
