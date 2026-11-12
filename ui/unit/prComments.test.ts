// PR comment helpers (modules/git/pr-comments.ts): the reaction toggle the
// pills apply optimistically, edit/delete patches, the "replying to" label for
// flattened Bitbucket chains, and the GitHub review-summary chip.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { findComment, isReviewSummary, mapComment, replyingTo, reviewChip, toggleReaction } from '../src/modules/git/pr-comments.ts';
import type { PrComment } from '../src/lib/api/types';

const cm = (id: string, author: string, extra: Partial<PrComment> = {}): PrComment => ({
  id, author, body: id, path: null, line: null, created_at: '2026-11-12T10:00:00Z', replies: [], resolved: false, ...extra,
});

test('toggleReaction adds yours, bumps an existing one, and takes yours back', () => {
  assert.deepEqual(toggleReaction(undefined, 'heart'), [{ key: 'heart', count: 1, me: true }]);
  const others = [{ key: 'thumbs_up' as const, count: 2, me: false }];
  assert.deepEqual(toggleReaction(others, 'thumbs_up'), [{ key: 'thumbs_up', count: 3, me: true }]);
  assert.deepEqual(toggleReaction([{ key: 'thumbs_up', count: 3, me: true }], 'thumbs_up'), [{ key: 'thumbs_up', count: 2, me: false }]);
  // Only yours: the pill disappears; the others keep their order.
  assert.deepEqual(
    toggleReaction([{ key: 'eyes', count: 1, me: false }, { key: 'rocket', count: 1, me: true }, { key: 'laugh', count: 4, me: false }], 'rocket'),
    [{ key: 'eyes', count: 1, me: false }, { key: 'laugh', count: 4, me: false }],
  );
  // Toggling twice is a round trip.
  assert.deepEqual(toggleReaction(toggleReaction(others, 'thumbs_up'), 'thumbs_up'), others);
});

test('mapComment edits or removes a head or a reply; findComment reads either', () => {
  const list = [cm('a', 'ann', { replies: [cm('a1', 'bob'), cm('a2', 'ann')] }), cm('b', 'bob')];
  const edited = mapComment(list, 'a2', (c) => ({ ...c, body: 'new' }));
  assert.equal(findComment(edited, 'a2')?.body, 'new');
  assert.equal(list[0].replies[1].body, 'a2', 'input untouched');
  assert.deepEqual(mapComment(list, 'a1', () => null)[0].replies.map((r) => r.id), ['a2']);
  assert.deepEqual(mapComment(list, 'a', () => null).map((c) => c.id), ['b']);
  assert.equal(mapComment(list, 'b', (c) => c)[0], list[0], 'untouched threads keep their identity');
  assert.equal(findComment(list, 'zz'), null);
});

test('replyingTo names the reply being answered, never the head', () => {
  const root = cm('h', 'ann', { thread_id: 'h', replies: [cm('r1', 'bob', { in_reply_to: 'h' }), cm('r2', 'ann', { in_reply_to: 'r1' })] });
  assert.equal(replyingTo(root.replies[0], root), null);
  assert.equal(replyingTo(root.replies[1], root), 'bob');
  assert.equal(replyingTo(cm('r3', 'x'), root), null, 'no in_reply_to (GitHub, older daemon)');
  assert.equal(replyingTo(cm('r4', 'x', { in_reply_to: 'gone' }), root), null, 'unknown target');
});

test('GitHub review summaries: chip per verdict, recognised by id or state', () => {
  assert.deepEqual(reviewChip('approved'), { label: 'Approved', tone: 'success' });
  assert.deepEqual(reviewChip('changes_requested'), { label: 'Changes requested', tone: 'warning' });
  assert.equal(reviewChip('commented').label, 'Commented');
  assert.equal(reviewChip('dismissed').label, 'Dismissed');
  assert.ok(isReviewSummary(cm('review-9', 'x')));
  assert.ok(isReviewSummary(cm('77', 'x', { review_state: 'commented' })));
  assert.ok(!isReviewSummary(cm('77', 'x')));
});
