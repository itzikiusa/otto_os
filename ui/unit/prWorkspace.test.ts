import { test } from 'node:test';
import assert from 'node:assert/strict';
import { prTabsAfterClose, prTabsAfterOpen } from '../src/modules/git/prTabs.ts';
import { buildTimeline } from '../src/modules/git/prTimeline.ts';
import type { PrCommit, PrDetail } from '../src/lib/api/types.ts';

test('foreground open fronts the PR; reopening keeps tab order', () => {
  let t = prTabsAfterOpen({ open: [], active: null }, [5], false, 12);
  assert.deepEqual(t, { open: [5], active: 5 });
  t = prTabsAfterOpen(t, [7], false, 12);
  assert.deepEqual(t, { open: [5, 7], active: 7 });
  t = prTabsAfterOpen(t, [5], false, 12);
  assert.deepEqual(t, { open: [5, 7], active: 5 });
});

test('background open keeps the front tab, unless nothing is open', () => {
  assert.deepEqual(prTabsAfterOpen({ open: [1], active: 1 }, [2], true, 12), { open: [1, 2], active: 1 });
  assert.deepEqual(prTabsAfterOpen({ open: [], active: null }, [2], true, 12), { open: [2], active: 2 });
});

test('open many fronts the first and trims the oldest background tabs past max', () => {
  const t = prTabsAfterOpen({ open: [1, 2, 3], active: 2 }, [4, 5], false, 4);
  assert.deepEqual(t, { open: [2, 3, 4, 5], active: 4 });
});

test('closing the front tab moves to the right neighbour, then the left', () => {
  assert.deepEqual(prTabsAfterClose({ open: [1, 2, 3], active: 2 }, 2), { open: [1, 3], active: 3 });
  assert.deepEqual(prTabsAfterClose({ open: [1, 2, 3], active: 3 }, 3), { open: [1, 2], active: 2 });
  assert.deepEqual(prTabsAfterClose({ open: [1], active: 1 }, 1), { open: [], active: null });
  assert.deepEqual(prTabsAfterClose({ open: [1, 2], active: 1 }, 2), { open: [1], active: 1 });
  const same = { open: [1], active: 1 };
  assert.equal(prTabsAfterClose(same, 9), same);
});

const commit = (sha: string, author: string, date: string): PrCommit => ({ sha, short_sha: sha, author, date, subject: sha });
const basePr = (over: Partial<PrDetail> = {}): PrDetail => ({
  number: 1,
  title: 't',
  author: 'ana',
  state: 'open',
  source_branch: 'f',
  target_branch: 'main',
  updated_at: '2026-01-05T00:00:00Z',
  created_at: '2026-01-01T00:00:00Z',
  url: '',
  description_md: '',
  comments: [],
  approved_by: [],
  reviewers: [],
  mergeable: null,
  ...over,
});

test('timeline: opened, folded commit bursts, comments and approvals in order', () => {
  const pr = basePr({
    comments: [
      { id: 'c1', author: 'bo', body: 'hi', path: null, line: null, created_at: '2026-01-02T12:00:00Z', replies: [], resolved: false },
      { id: 'c2', author: 'bo', body: 'x', path: null, line: null, created_at: 'not a date', replies: [], resolved: false },
    ],
    reviewers: [
      { name: 'bo', approved: true, changes_requested: false, avatar_url: null, reviewed_at: '2026-01-03T00:00:00Z' },
      { name: 'cy', approved: true, changes_requested: false, avatar_url: null, reviewed_at: null },
    ],
  });
  const ev = buildTimeline(pr, [
    commit('a', 'ana', '2026-01-02T10:00:00Z'),
    commit('b', 'ana', '2026-01-02T10:10:00Z'),
    commit('c', 'bo', '2026-01-02T10:15:00Z'),
    commit('d', 'ana', '2026-01-04T00:00:00Z'),
  ]);
  assert.deepEqual(
    ev.map((e) => `${e.kind}:${e.who}${e.kind === 'commits' ? `×${e.commits.length}` : ''}`),
    ['opened:ana', 'commits:ana×2', 'commits:bo×1', 'comment:bo', 'approved:bo', 'commits:ana×1'],
  );
});

test('timeline: merged PR ends with a closed event; no created_at → no opened event', () => {
  const ev = buildTimeline(basePr({ created_at: null, state: 'merged' }), null);
  assert.deepEqual(ev.map((e) => e.kind), ['closed']);
});
