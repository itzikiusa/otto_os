'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { createPacer } = require('../lib/pacer');
const { createPrClient, derivePrMetrics, prFlowSummary, extractKeys, normalizePr, cacheFile } = require('../lib/prs');
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
  assert.deepEqual(derivePrMetrics(pr), {
    pickup_days: 1, review_days: 0, merge_lag_days: 1, size: 150, size_bucket: '<200', review_depth: 0, unreviewed: false,
  });
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
