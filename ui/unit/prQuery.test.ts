// PR list search language, sort, grouping and refresh merge
// (modules/git/pr-query.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  groupPrs,
  hasClientFilter,
  matchesPr,
  mergeRefreshed,
  parsePrQuery,
  sortPrs,
  tokenize,
} from '../src/modules/git/pr-query.ts';
import type { PrSummary } from '../src/lib/api/types';

const pr = (n: number, o: Partial<PrSummary> = {}): PrSummary => ({
  number: n,
  title: `PR ${n}`,
  author: 'alice',
  state: 'open',
  source_branch: `feature/${n}`,
  target_branch: 'develop',
  updated_at: `2026-11-0${(n % 9) + 1}T00:00:00Z`,
  url: '',
  labels: [],
  ...o,
});

test('tokenize keeps quoted values whole, also after a qualifier', () => {
  assert.deepEqual(tokenize('label:"needs review" fix  "two words"'), ['label:needs review', 'fix', 'two words']);
});

test('qualifiers parse; unknown ones stay free text', () => {
  const q = parsePrQuery('Fix author:Bob label:bug base:main head:feat is:merged -is:draft ci:failing #12 foo:bar');
  assert.deepEqual(q.text, ['fix', 'foo:bar']);
  assert.deepEqual(q.authors, ['bob']);
  assert.deepEqual(q.labels, ['bug']);
  assert.deepEqual(q.base, ['main']);
  assert.deepEqual(q.head, ['feat']);
  assert.equal(q.state, 'merged');
  assert.equal(q.draft, false);
  assert.equal(q.ci, 'failing');
  assert.equal(q.number, 12);
});

test('is:closed maps to declined; author:@me and is:mine set mine', () => {
  assert.equal(parsePrQuery('is:closed').state, 'declined');
  assert.equal(parsePrQuery('author:@me').mine, true);
  assert.equal(parsePrQuery('is:mine').mine, true);
  assert.equal(parsePrQuery('is:draft').draft, true);
});

test('a state qualifier alone is not a client filter', () => {
  assert.equal(hasClientFilter(parsePrQuery('is:all')), false);
  assert.equal(hasClientFilter(parsePrQuery('x')), true);
});

test('matching: text across fields, labels, numbers, mine, ci, drafts', () => {
  const me = new Set(['alice']);
  const a = pr(7, { title: 'Add VIP screens', labels: ['bug', 'UI'], ci_status: 'failure', draft: true });
  assert.ok(matchesPr(a, parsePrQuery('vip'), me));
  assert.ok(matchesPr(a, parsePrQuery('7'), me));
  assert.ok(matchesPr(a, parsePrQuery('#7'), me));
  assert.ok(!matchesPr(a, parsePrQuery('#70'), me));
  assert.ok(matchesPr(a, parsePrQuery('label:ui label:BUG'), me));
  assert.ok(!matchesPr(a, parsePrQuery('label:docs'), me));
  assert.ok(matchesPr(a, parsePrQuery('ci:failing is:draft is:mine'), me));
  assert.ok(!matchesPr(a, parsePrQuery('-is:draft'), me));
  assert.ok(!matchesPr(a, parsePrQuery('is:mine'), new Set(['bob'])));
  assert.ok(matchesPr(a, parsePrQuery('head:feature/7 base:dev'), me));
  assert.ok(!matchesPr(a, parsePrQuery('vip missing'), me));
});

test('sorts: updated, newest falls back to number, changed uses size', () => {
  const rows = [pr(1, { updated_at: '2026-11-01T00:00:00Z' }), pr(2, { updated_at: '2026-11-03T00:00:00Z' }), pr(3, { updated_at: '2026-11-02T00:00:00Z' })];
  assert.deepEqual(sortPrs(rows, 'updated').map((p) => p.number), [2, 3, 1]);
  assert.deepEqual(sortPrs(rows, 'newest').map((p) => p.number), [3, 2, 1]);
  assert.deepEqual(sortPrs(rows, 'oldest').map((p) => p.number), [1, 2, 3]);
  const size = (p: PrSummary) => (p.number === 1 ? 500 : p.number === 3 ? 10 : null);
  assert.deepEqual(sortPrs(rows, 'changed', size).map((p) => p.number), [1, 3, 2]);
  assert.deepEqual(rows.map((p) => p.number), [1, 2, 3], 'input untouched');
});

test('groups: mine first, flat without handles', () => {
  const rows = [pr(1, { author: 'Bob' }), pr(2, { author: 'ALICE' })];
  assert.deepEqual(groupPrs(rows, new Set()).map((g) => g.id), ['all']);
  const g = groupPrs(rows, new Set(['alice']));
  assert.deepEqual(g.map((x) => [x.id, x.items.map((p) => p.number)]), [['mine', [2]], ['others', [1]]]);
  assert.deepEqual(groupPrs(rows, new Set(['carol'])).map((x) => x.id), ['all']);
});

test('refresh merge replaces page 1 and keeps later pages', () => {
  const prev = [pr(5), pr(4), pr(3), pr(2), pr(1)];
  // page 1 had 3 rows; #4 was merged away, #6 is new, #3 updated.
  const fresh = [pr(6), pr(5), pr(3, { title: 'new' })];
  const out = mergeRefreshed(prev, fresh, 3);
  assert.deepEqual(out.map((p) => p.number), [6, 5, 3, 2, 1]);
  assert.equal(out[2].title, 'new');
});
