// Merging what you didn't read (modules/git/pr-merge-gate.ts): the merge
// dialog compares the head it would merge with the head the diff view was
// loaded at — and the provider-accurate strategy labels (pr-merge-strategy.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { sameHead, unseenHead, unseenText } from '../src/modules/git/pr-merge-gate.ts';
import { mergeStrategies, strategyMenuLabel } from '../src/modules/git/pr-merge-strategy.ts';
import type { PrCommit } from '../src/lib/api/types';

const c = (sha: string, mins: number): PrCommit => ({
  sha: `${sha}${'0'.repeat(32)}`,
  short_sha: sha.slice(0, 7),
  author: 'maya',
  date: new Date(Date.UTC(2026, 10, 12, 12, 0) - mins * 60_000).toISOString(),
  subject: sha,
});

test('heads compare across Bitbucket abbreviations, never on empty', () => {
  assert.ok(sameHead('c1fac680', 'c1fac6800000000000000000000000000000000'));
  assert.ok(sameHead('C1FAC680', 'c1fac680'));
  assert.ok(!sameHead('c1fac680', 'c7e57a11'));
  assert.ok(!sameHead('', ''));
});

test('nothing to warn about: same head, or no diff was ever loaded', () => {
  assert.deepEqual(unseenHead('c1fac680', 'c1fac680aaaa', null), { moved: false, count: null });
  assert.deepEqual(unseenHead('c1fac680', '', null), { moved: false, count: null });
  assert.deepEqual(unseenHead('c1fac680', null, null), { moved: false, count: null });
  assert.deepEqual(unseenHead(null, 'c1fac680', null), { moved: false, count: null });
});

test('a push after the diff was read is caught, counted from the commits (any order)', () => {
  const commits = [c('a3333333', 1), c('c1fac680', 30), c('a2222222', 5), c('b0000000', 90)];
  const u = unseenHead('a3333333', 'c1fac680', commits);
  assert.deepEqual(u, { moved: true, count: 2 });
  assert.equal(unseenText(u), '2 new commits since you opened this diff');
  assert.equal(unseenText(unseenHead('a3333333', 'a2222222', commits)), '1 new commit since you opened this diff');
});

test('unknown count: commits not loaded, or the read commit was rewritten away', () => {
  assert.deepEqual(unseenHead('a3333333', 'c1fac680', null), { moved: true, count: null });
  const u = unseenHead('a3333333', 'deadbeef', [c('a3333333', 1)]);
  assert.deepEqual(u, { moved: true, count: null });
  assert.equal(unseenText(u), 'New commits since you opened this diff');
});

test('Bitbucket calls its "rebase" what it is: a fast-forward', () => {
  assert.deepEqual(mergeStrategies('bitbucket').map((o) => [o.value, o.label]), [['merge', 'Merge commit'], ['squash', 'Squash'], ['rebase', 'Fast-forward']]);
  assert.deepEqual(mergeStrategies('github').map((o) => o.label), ['Merge commit', 'Squash', 'Rebase']);
  assert.deepEqual(mergeStrategies(null).map((o) => o.label), ['Merge commit', 'Squash', 'Rebase']);
  assert.equal(strategyMenuLabel('bitbucket', 'rebase'), 'Fast-forward merge');
  assert.equal(strategyMenuLabel('gitlab', 'rebase'), 'Rebase and merge');
  assert.equal(strategyMenuLabel('bitbucket', 'squash'), 'Squash and merge');
});
