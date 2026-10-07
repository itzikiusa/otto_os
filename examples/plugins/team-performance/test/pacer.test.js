'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { createPacer, parseRetryAfter } = require('../lib/pacer');

function clock() {
  const c = { t: 0, sleeps: [] };
  c.now = () => c.t;
  c.sleep = async (ms) => { c.sleeps.push(ms); c.t += ms; };
  return c;
}
const ok = { status: 200 };
const r429 = (ra) => ({ status: 429, headers: { 'Retry-After': ra } });

test('calls are spaced at least minIntervalMs apart', async () => {
  const c = clock();
  const p = createPacer({ sleep: c.sleep, now: c.now });
  const starts = [];
  await Promise.all([1, 2, 3, 4].map(() => p.schedule(async () => { starts.push(c.t); return ok; })));
  assert.deepEqual(starts, [0, 2000, 4000, 6000]);
});

test('429 with Retry-After: 3 waits 3s before retrying', async () => {
  const c = clock();
  const p = createPacer({ sleep: c.sleep, now: c.now });
  const starts = [];
  let n = 0;
  const res = await p.schedule(async () => { starts.push(c.t); return n++ === 0 ? r429('3') : ok; });
  assert.equal(res.status, 200);
  assert.deepEqual(starts, [0, 3000]);
  assert.ok(c.sleeps.includes(3000));
});

test('repeated 429 backs off exponentially, capped, then gives up', async () => {
  const c = clock();
  const p = createPacer({ sleep: c.sleep, now: c.now, minIntervalMs: 0, baseBackoffMs: 2000, capMs: 10000, maxRetries: 5 });
  let attempts = 0;
  const res = await p.schedule(async () => { attempts++; return { status: 429, headers: {} }; });
  assert.equal(res.status, 429);
  assert.equal(attempts, 6);
  assert.deepEqual(c.sleeps, [2000, 4000, 8000, 10000, 10000]);
});

test('5xx gets a limited retry budget', async () => {
  const c = clock();
  const p = createPacer({ sleep: c.sleep, now: c.now, minIntervalMs: 0, max5xxRetries: 2 });
  let attempts = 0;
  const res = await p.schedule(async () => { attempts++; return { status: 503 }; });
  assert.equal(res.status, 503);
  assert.equal(attempts, 3);
});

test('network errors retry then rethrow; queue keeps serving', async () => {
  const c = clock();
  const p = createPacer({ sleep: c.sleep, now: c.now, minIntervalMs: 0, max5xxRetries: 1 });
  await assert.rejects(p.schedule(async () => { throw new Error('ECONNRESET'); }), /ECONNRESET/);
  assert.equal((await p.schedule(async () => ok)).status, 200);
});

test('parseRetryAfter handles seconds and HTTP dates', () => {
  assert.equal(parseRetryAfter('3', 0), 3000);
  assert.equal(parseRetryAfter(new Date(5000).toUTCString(), 0), 5000);
  assert.equal(parseRetryAfter(null, 0), null);
});

// ---- paced ingestion through the PR client (fake clock, mock daemon) ----
const fs = require('fs');
const os = require('os');
const path = require('path');
const { createPrClient, cacheFile, prIngestStatus } = require('../lib/prs');
const { createMockDaemon, res: mkRes } = require('./fixtures/mock-daemon-prs');

const tmpDir = () => fs.mkdtempSync(path.join(os.tmpdir(), 'tp-pacer-'));
const pr = (n, updated) => ({
  summary: { number: n, title: `ABC-${n}`, author: 'dev', state: 'merged', updated_at: updated },
  detail: { created_at: '2026-01-05T10:00:00Z', merged_at: updated, comments: [], reviewers: [] },
  commits: [], diff: { files: [] },
});
const detailCalls = (calls) => calls.filter((x) => /\/prs\/\d+$/.test(x.route)).map((x) => x.route.match(/(\d+)$/)[1]);

function setup(prs, wrap) {
  const c = clock();
  const dataDir = tmpDir();
  const daemon = createMockDaemon({ prs });
  const starts = [];
  const fetchImpl = (url, init) => { starts.push(c.t); return wrap ? wrap(url, init, daemon) : daemon.fetch(url, init); };
  const warns = [];
  const pacer = createPacer({ sleep: c.sleep, now: c.now });
  const client = createPrClient({ baseUrl: 'http://d/api/v1', fetchImpl, dataDir, pacer, onWarn: (w) => warns.push(w) });
  return { c, dataDir, daemon, starts, client, warns, pacer };
}

test('one pacer keeps >= 2000ms between every call across two repos', async () => {
  const s = setup({ r1: [pr(1, '2026-01-07T10:00:00Z'), pr(2, '2026-01-08T10:00:00Z')], r2: [pr(5, '2026-01-07T10:00:00Z')] });
  await Promise.all([s.client.syncRepo('r1'), s.client.syncRepo('r2')]);
  assert.ok(s.starts.length >= 8);
  for (let i = 1; i < s.starts.length; i++) assert.ok(s.starts[i] - s.starts[i - 1] >= 2000, `gap ${i}`);
});

test('persistent 429 exhausting retries fails the sync and leaves the cursor unchanged', async () => {
  let throttle = false;
  const s = setup({ r1: [pr(1, '2026-01-07T10:00:00Z')] }, (url, init, d) =>
    (throttle ? mkRes(429, {}, { 'retry-after': '1' }) : d.fetch(url, init)));
  await s.client.syncRepo('r1');
  const before = JSON.parse(fs.readFileSync(cacheFile(s.dataDir, 'r1'), 'utf8')).cursor;
  s.daemon.state.prs.r1.push(pr(2, '2026-01-09T10:00:00Z'));
  throttle = true;
  await assert.rejects(s.client.syncRepo('r1'), (e) => e.status === 429);
  const after = JSON.parse(fs.readFileSync(cacheFile(s.dataDir, 'r1'), 'utf8'));
  assert.deepEqual(after.cursor, before);
  assert.match(after.meta.last_error, /429/);
  assert.equal(s.pacer.stats.throttled, 6); // 1 + maxRetries(5)
  const st = prIngestStatus({ dataDir: s.dataDir, configured: true, repoIds: ['r1'] });
  assert.equal(st.state, 'failing'); // error recorded after the last good sync
  assert.match(st.last_error, /429/);
});

test('corrupt cache file is recovered as empty and logged', async () => {
  const s = setup({ r1: [pr(1, '2026-01-07T10:00:00Z')] });
  fs.mkdirSync(path.dirname(cacheFile(s.dataDir, 'r1')), { recursive: true });
  fs.writeFileSync(cacheFile(s.dataDir, 'r1'), '{not json');
  const r = await s.client.syncRepo('r1');
  assert.equal(r.fetched, 1);
  assert.equal(s.warns.length, 1);
  assert.equal(s.warns[0].repo, 'r1');
  assert.ok(JSON.parse(fs.readFileSync(cacheFile(s.dataDir, 'r1'), 'utf8')).cursor.synced_at);
});

test('updated_on newer than the cursor refetches detail; an unchanged second run makes zero detail calls', async () => {
  const s = setup({ r1: [pr(1, '2026-01-07T10:00:00Z'), pr(2, '2026-01-08T10:00:00Z')] });
  await s.client.syncRepo('r1');
  s.daemon.calls.length = 0;
  s.daemon.state.prs.r1[0].summary.updated_at = '2026-01-10T10:00:00Z';
  const r2 = await s.client.syncRepo('r1');
  assert.deepEqual(detailCalls(s.daemon.calls), ['1']);
  assert.equal(r2.fetched, 1);
  s.daemon.calls.length = 0;
  const r3 = await s.client.syncRepo('r1');
  assert.equal(r3.fetched, 0);
  assert.deepEqual(detailCalls(s.daemon.calls), []);
});

test('quality: abort interrupts backoff and prevents queued dispatch', async () => {
  const controller = new AbortController();
  let calls = 0;
  let waiting;
  const ready = new Promise((r) => { waiting = r; });
  const pacer = createPacer({ minIntervalMs: 0, onWait: waiting, sleep: () => new Promise(() => {}) });
  const first = pacer.schedule(async () => { calls++; return { status: 429, headers: { 'retry-after': '30' } }; }, { signal: controller.signal });
  const second = pacer.schedule(async () => { calls++; return { status: 200 }; }, { signal: controller.signal });
  const outcomes = Promise.allSettled([first, second]);
  await ready;
  controller.abort();
  const result = await Promise.race([outcomes, new Promise((r) => setTimeout(() => r(null), 300))]);
  assert.ok(result, 'Stop must interrupt the rate-limit wait');
  assert.equal(calls, 1);
  assert.ok(result.every((r) => r.status === 'rejected' && r.reason.name === 'AbortError'));
});

test('quality edge: cancelled account exits a shared queue behind another active account', async () => {
  let release;
  const active = new Promise((r) => { release = r; });
  const pacer = createPacer({ minIntervalMs: 0 });
  const first = pacer.schedule(() => active);
  const controller = new AbortController();
  let dispatched = false;
  const queued = pacer.schedule(() => { dispatched = true; return ok; }, { signal: controller.signal });
  const outcome = queued.then(() => 'resolved', (e) => e.name);
  try {
    controller.abort();
    assert.equal(await Promise.race([outcome, new Promise((r) => setTimeout(() => r('hung'), 100))]), 'AbortError');
    assert.equal(dispatched, false);
  } finally { release(ok); await first; await outcome; }
});
