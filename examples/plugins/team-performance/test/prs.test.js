'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { createPacer } = require('../lib/pacer');
const { createPrClient, derivePrMetrics, prFlowSummary, prSummaryByKey, prIngestStatus, extractKeys, normalizePr, cacheFile } = require('../lib/prs');
const { createMockDaemon } = require('./fixtures/mock-daemon-prs');

function clock() {
  const c = { t: Date.parse('2026-01-01T00:00:00Z'), sleeps: [] };
  c.now = () => c.t;
  c.sleep = async (ms) => { c.sleeps.push(ms); c.t += ms; };
  return c;
}

function mkPr(n, updated, extra = {}) {
  return {
    summary: { number: n, title: `ABC-${n} do thing`, author: 'dev', state: 'merged', source_branch: `feature/ABC-${n}-x`,
      target_branch: 'develop', updated_at: updated, url: `https://example.test/pr/${n}` },
    detail: { created_at: '2026-01-05T10:00:00Z', merged_at: updated, merge_commit_sha: `sha${n}`,
      comments: [{ id: '1', author: 'rev', body: 'nit', created_at: '2026-01-05T15:00:00Z', replies: [] }],
      reviewers: [{ name: 'rev', approved: true, reviewed_at: '2026-01-06T10:00:00Z' }] },
    commits: [{ sha: `c${n}`, date: '2026-01-05T09:00:00Z' }],
    diff: { files: [{ added: 30, deleted: 10 }], total_added: 30, total_deleted: 10 },
    ...extra,
  };
}

const tmp = () => fs.mkdtempSync(path.join(os.tmpdir(), 'tp-prs-'));

test('sync through the mock daemon: paced, 429 honoured, cached; second sync is incremental and survives a new client', async () => {
  const dataDir = tmp();
  const c = clock();
  const daemon = createMockDaemon({ throttleFirst: true, prs: { r1: [mkPr(1, '2026-01-07T10:00:00Z'), mkPr(2, '2026-01-08T10:00:00Z')] } });
  const starts = [];
  const fetchImpl = (url, init) => { starts.push(c.t); return daemon.fetch(url, init); };
  const mk = () => createPrClient({ baseUrl: 'http://d/api/v1', token: 'T', fetchImpl, dataDir,
    pacer: createPacer({ sleep: c.sleep, now: c.now }) });
  const progress = [];
  const client = createPrClient({ baseUrl: 'http://d/api/v1', token: 'T', fetchImpl, dataDir,
    pacer: createPacer({ sleep: c.sleep, now: c.now }), onProgress: (p) => progress.push(p) });

  const r1 = await client.syncRepo('r1');
  assert.equal(r1.fetched, 2);
  assert.equal(daemon.calls[0].auth, 'Bearer T');
  for (let i = 1; i < starts.length; i++) assert.ok(starts[i] - starts[i - 1] >= 2000, `gap ${i}`);
  assert.equal(starts[1] - starts[0], 3000); // Retry-After: 3
  assert.ok(progress.length && progress.every((p) => p.repo === 'r1' && 'next_call_eta_ms' in p));

  const saved = JSON.parse(fs.readFileSync(cacheFile(dataDir, 'r1'), 'utf8'));
  assert.equal(saved.cursor.updated_on_max, '2026-01-08T10:00:00.000Z');
  assert.equal(saved.prs['1'].merge_commit, 'sha1');
  assert.deepEqual(saved.prs['1'].keys, ['ABC-1']);
  assert.ok(!fs.readdirSync(path.dirname(cacheFile(dataDir, 'r1'))).some((f) => f.includes('.tmp')));

  // New PR + one updated; a fresh client must reuse the persisted cursor.
  daemon.state.prs.r1.push(mkPr(3, '2026-01-09T10:00:00Z'));
  daemon.state.prs.r1[0].summary.updated_at = '2026-01-10T10:00:00Z';
  daemon.calls.length = 0;
  const r2 = await mk().syncRepo('r1');
  assert.equal(r2.fetched, 2);
  const detailNums = daemon.calls.map((x) => x.route.match(/\/prs\/(\d+)$/)).filter(Boolean).map((m) => m[1]).sort();
  assert.deepEqual(detailNums, ['1', '3']);
  assert.equal(r2.total, 3);

  daemon.calls.length = 0;
  const r3 = await mk().syncRepo('r1');
  assert.equal(r3.fetched, 0);
  assert.equal(daemon.calls.length, 1); // one list page, no details
});

test('mapRepoPaths maps local paths to daemon repo ids', async () => {
  const c = clock();
  const daemon = createMockDaemon({ repos: [{ id: 'R1', name: 'svc', path: '/src/svc' }] });
  const client = createPrClient({ baseUrl: 'http://d/api/v1', fetchImpl: daemon.fetch, dataDir: tmp(),
    pacer: createPacer({ sleep: c.sleep, now: c.now }) });
  assert.deepEqual(await client.mapRepoPaths(['/src/svc/', '/src/other']), { '/src/svc/': 'R1', '/src/other': null });
});

test('derivePrMetrics: opened Mon 10:00, approved Tue 10:00, merged Wed 10:00', () => {
  const pr = normalizePr(
    { number: 7, title: 'ABC-1 fix', author: 'dev', state: 'MERGED', source_branch: 'ABC-1', updated_at: '2026-01-07T10:00:00Z' },
    { created_at: '2026-01-05T10:00:00Z', merged_at: '2026-01-07T10:00:00Z', comments: [],
      reviewers: [{ name: 'rev', approved: true, reviewed_at: '2026-01-06T10:00:00Z' }] },
    [], { files: [{ added: 120, deleted: 30 }] });
  assert.equal(pr.first_review_at, '2026-01-06T10:00:00.000Z');
  const m = derivePrMetrics(pr);
  assert.deepEqual(
    { pickup_days: m.pickup_days, review_days: m.review_days, merge_lag_days: m.merge_lag_days, size: m.size,
      size_bucket: m.size_bucket, review_depth: m.review_depth, unreviewed: m.unreviewed },
    { pickup_days: 1, review_days: 0, merge_lag_days: 1, size: 150, size_bucket: '<200', review_depth: 0, unreviewed: false });
  assert.equal(m.pr_cycle_days, 2);
  assert.equal(m.merged_without_approval, false);
  assert.equal(m.coding_days, null); // no commits → unknown, not 0
});

test('business days skip the weekend; unreviewed + review depth', () => {
  const pr = { opened_at: '2026-01-09T10:00:00Z', merged_at: '2026-01-12T10:00:00Z', additions: 400, deletions: 0,
    first_review_at: null, first_approval_at: null, last_approval_at: null, reviewer_comment_count: 0 };
  const m = derivePrMetrics(pr);
  assert.equal(m.pickup_days, null);
  assert.equal(m.size_bucket, '>=400');
  assert.equal(m.unreviewed, true);
  const m2 = derivePrMetrics({ ...pr, first_review_at: '2026-01-12T10:00:00Z', reviewer_comment_count: 4 });
  assert.equal(m2.pickup_days, 1); // Fri 10:00 → Mon 10:00
  assert.equal(m2.review_depth, 1);
});

test('prFlowSummary: percentiles with n, null when empty', () => {
  assert.deepEqual(prFlowSummary([]).pickup_days, { value: null, n: 0 });
  const base = { opened_at: '2026-01-05T10:00:00Z', merged_at: '2026-01-09T10:00:00Z', additions: 10, deletions: 0 };
  const prs = [1, 2, 3].map((d) => ({ ...base, first_review_at: `2026-01-0${5 + d}T10:00:00Z` }));
  const s = prFlowSummary(prs);
  assert.deepEqual(s.pickup_days, { p50: 2, p75: 2.5, n: 3 });
  assert.equal(s.size_buckets['<50'], 3);
});

test('extractKeys pulls generic keys from branch and title', () => {
  assert.deepEqual(extractKeys('feature/ABC-1-x', 'ABC-1 and XY2-30 follow-up'), ['ABC-1', 'XY2-30']);
});

test('normalizePr keeps reviewer ids, approvals, comment authors, commit timestamps and a normalized state', () => {
  const pr = normalizePr(
    { number: 9, title: 'ABC-9', author: 'dev', state: 'DECLINED', updated_at: '2026-01-08T10:00:00Z' },
    { created_at: '2026-01-05T10:00:00Z',
      comments: [{ author: 'rev', created_at: '2026-01-05T12:00:00Z', replies: [{ author: 'dev', created_at: '2026-01-05T13:00:00Z' }, { author: 'rev', created_at: '2026-01-05T14:00:00Z' }] }],
      reviewers: [{ name: 'rev', id: 'u-1', approved: true, reviewed_at: '2026-01-06T10:00:00Z' }, { name: 'lead', approved: false }] },
    [{ sha: 'a', date: '2026-01-04T10:00:00Z' }, { sha: 'b', date: '2026-01-02T10:00:00Z' }], null);
  assert.equal(pr.state, 'declined');
  assert.equal(pr.state_raw, 'declined');
  assert.deepEqual(pr.reviewer_ids, ['u-1', 'lead']);
  assert.deepEqual(pr.approvers, ['rev']);
  assert.equal(pr.approvals_count, 1);
  assert.deepEqual(pr.comment_authors, { rev: 2, dev: 1 });
  assert.equal(pr.comment_count, 3);
  assert.deepEqual(pr.commit_dates, ['2026-01-02T10:00:00.000Z', '2026-01-04T10:00:00.000Z']);
  assert.equal(pr.first_commit_at, '2026-01-02T10:00:00.000Z');
  assert.equal(pr.merged_at, null);
  assert.equal(normalizePr({ number: 1, state: 'OPEN' }, {}, [], null).state, 'open');
});

const richPr = () => ({
  number: 1, author: 'dev', state: 'merged', keys: ['ABC-1'], url: 'u',
  first_commit_at: '2026-01-05T08:00:00Z', // Mon
  commit_dates: ['2026-01-05T08:00:00Z', '2026-01-06T12:00:00Z', '2026-01-07T09:00:00Z'],
  opened_at: '2026-01-05T10:00:00Z', first_review_at: '2026-01-06T10:00:00Z',
  review_event_dates: ['2026-01-06T10:00:00Z', '2026-01-06T11:00:00Z', '2026-01-07T10:00:00Z'],
  first_approval_at: null, last_approval_at: null, merged_at: '2026-01-08T10:00:00Z',
  additions: 60, deletions: 0, comment_count: 4, reviewer_comment_count: 3,
  comment_authors: { rev: 3, dev: 1 }, reviewers: [{ name: 'lead', approved: false }],
});

test('derivePrMetrics: coding, cycle, comments, reviewers, post-review commits, rounds, merged without approval', () => {
  const m = derivePrMetrics(richPr());
  assert.equal(m.coding_days, 0.08); // 2h
  assert.equal(m.pr_cycle_days, 3);
  assert.equal(m.comments_count, 4);
  assert.equal(m.reviewers_n, 2); // lead + rev (comment author), author excluded
  assert.equal(m.post_review_commits, 2);
  assert.equal(m.review_rounds, 2); // review burst, push, review again, push (no reviewer return)
  assert.equal(m.merged_without_approval, true);
});

test('prFlowSummary: new distributions, by_person with capacity rates, review load', () => {
  const a = richPr();
  const b = { ...richPr(), number: 2, author: 'rev', comment_authors: { dev: 2 }, reviewers: [{ name: 'dev', approved: true }],
    first_approval_at: '2026-01-07T10:00:00Z', last_approval_at: '2026-01-07T10:00:00Z' };
  const open = { number: 3, author: 'dev', state: 'open', reviewers: [], comment_authors: {} };
  const s = prFlowSummary([a, b, open], { capacityDays: { dev: 10, rev: 0 } });
  assert.deepEqual(s.pr_cycle_days, { p50: 3, p75: 3, n: 2 });
  assert.equal(s.coding_days.n, 2);
  assert.equal(s.comments_count.p50, 4);
  assert.deepEqual(s.counts, { total: 3, merged: 2, declined: 0, open: 1 });
  assert.equal(s.merged_without_approval.count, 1);
  assert.deepEqual(s.by_person.dev, { authored: 2, merged: 1, reviewed_given: 1, comments_given: 2, approvals_given: 1,
    capacity_days: 10, per_capacity_day: { authored: 0.2, reviewed_given: 0.1, comments_given: 0.2 } });
  assert.equal(s.by_person.rev.per_capacity_day, null); // zero capacity → no rate
  assert.equal(s.by_person.lead.reviewed_given, 1);
  assert.equal(s.review_load.n, 3);
  assert.equal(s.review_load.reviewers_n, 3);
  assert.equal(prFlowSummary([]).review_load.value, null);
  assert.equal(prFlowSummary([]).by_person.dev, undefined);
});

test('prSummaryByKey is compact and carries no raw text', () => {
  const pr = { ...richPr(), title: 'secret title', body: 'raw body' };
  const out = prSummaryByKey([pr]);
  assert.deepEqual(Object.keys(out), ['ABC-1']);
  const json = JSON.stringify(out);
  assert.ok(!json.includes('secret title') && !json.includes('raw body'));
  assert.equal(out['ABC-1'].prs[0].number, 1);
  assert.equal(out['ABC-1'].merged_without_approval, 1);
  assert.equal(out['ABC-1'].post_review_commits, 2);
});

test('prIngestStatus explains every empty state', () => {
  const dataDir = tmp();
  assert.equal(prIngestStatus({ dataDir, configured: false, repoIds: [] }).state, 'not_configured');
  const unreg = prIngestStatus({ dataDir, configured: true, repoIds: [], unregistered: ['/x'] });
  assert.equal(unreg.state, 'not_configured');
  assert.match(unreg.reason, /registered/);
  const never = prIngestStatus({ dataDir, configured: true, repoIds: ['r1'] });
  assert.equal(never.state, 'never_run');
  assert.deepEqual(never.pending, ['r1']);
  fs.mkdirSync(path.dirname(cacheFile(dataDir, 'r1')), { recursive: true });
  fs.writeFileSync(cacheFile(dataDir, 'r1'), JSON.stringify({ schema: 1, cursor: { updated_on_max: '2026-01-02T00:00:00.000Z', synced_at: '2026-01-03T00:00:00.000Z' }, prs: { 1: {} }, meta: {} }));
  const ok = prIngestStatus({ dataDir, configured: true, repoIds: ['r1'] });
  assert.equal(ok.state, 'ok');
  assert.equal(ok.fetched, 1);
  assert.equal(ok.cursor_by_repo.r1.updated_on_max, '2026-01-02T00:00:00.000Z');
  const failing = prIngestStatus({ dataDir, configured: true, repoIds: ['r1'], runtime: { repos: { r1: { ok: false, error: 'daemon 429' } } } });
  assert.equal(failing.state, 'failing');
  assert.equal(failing.last_error, 'daemon 429');
});
