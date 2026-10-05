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
