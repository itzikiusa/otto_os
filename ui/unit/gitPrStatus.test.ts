// A pull request in words (modules/git/prStatus.ts): one status line, one
// next step, the list row's word and the PR the list opens on.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { firstNeedingPr, prRowWord, prStatusLine } from '../src/modules/git/prStatus.ts';
import type { PrChecksResp, PrDetail, PrSummary } from '../src/lib/api/types';

const base = {
  number: 7, title: 'x', author: 'dana', state: 'open', source_branch: 'feat/x', target_branch: 'main',
  updated_at: '2026-11-24T00:00:00Z', url: '', description_md: '', comments: [], approved_by: [], reviewers: [], mergeable: true,
} as unknown as PrDetail;
const checks = (states: string[]): PrChecksResp => ({
  ci: { state: 'x', total: states.length, passed: states.filter((s) => s === 'success').length, failed: states.filter((s) => s === 'failure').length, url: null },
  checks: states.map((state, i) => ({ name: `c${i}`, state, url: state === 'failure' ? `https://ci/${i}` : null })) as PrChecksResp['checks'],
});

test('failing checks lead, with the count and the page "Fix checks" opens', () => {
  const s = prStatusLine(base, checks(['success', 'failure', 'failure', 'success']), null);
  assert.deepEqual(s, { words: 'Checks failing · 2 of 4', tone: 'danger', next: 'fix-checks', failingUrl: 'https://ci/1' });
});

test('then conflicts, changes requested, running checks, ready to merge, waiting', () => {
  assert.equal(prStatusLine({ ...base, mergeable: false }, null, null).words, 'Conflicts with main');
  const req = { ...base, reviewers: [{ name: 'Iris', approved: false, changes_requested: true, avatar_url: null, reviewed_at: null }] };
  assert.deepEqual(prStatusLine(req, null, null), { words: 'Changes requested by Iris', tone: 'warning', next: 'review', failingUrl: null });
  assert.equal(prStatusLine(base, checks(['pending', 'success']), null).words, 'Checks running · 1 of 2');
  const ok = { ...base, reviewers: [{ name: 'Iris', approved: true, changes_requested: false, avatar_url: null, reviewed_at: null }] };
  assert.deepEqual(prStatusLine(ok, checks(['success']), null), { words: 'Ready to merge', tone: 'success', next: 'merge', failingUrl: null });
  assert.equal(prStatusLine(base, null, null).next, 'review');
  assert.equal(prStatusLine(base, null, null).words, 'Waiting for review');
  assert.equal(prStatusLine({ ...base, state: 'merged' } as PrDetail, null, null).next, 'none');
});

test('list rows say what the PR needs; the list opens on what needs you', () => {
  const pr = (n: number, extra: Partial<PrSummary>): PrSummary => ({ ...(base as PrSummary), number: n, ...extra });
  assert.equal(prRowWord(pr(1, { ci_status: 'failure' })).word, 'Checks failing');
  assert.equal(prRowWord(pr(1, { review: { status: 'waiting', approvals: 0 } })).word, 'Review requested');
  assert.equal(prRowWord(pr(1, { state: 'merged' })).word, 'Merged');
  const list = [pr(1, {}), pr(2, { review: { status: 'waiting', approvals: 0 } }), pr(3, { ci_status: 'failure', is_mine: true })];
  assert.equal(firstNeedingPr(list, (p) => !!p.is_mine)?.number, 3);
  assert.equal(firstNeedingPr(list.slice(0, 2), (p) => !!p.is_mine)?.number, 2);
  assert.equal(firstNeedingPr([pr(9, {})], () => false)?.number, 9);
  assert.equal(firstNeedingPr([], () => false), null);
});
