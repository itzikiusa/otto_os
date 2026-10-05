// Scope cache: mtime-keyed LRU + per-entry memo.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs'); const os = require('os'); const path = require('path');
const { createScopeCache, scopeKey } = require('../lib/scopecache.js');

test('hits on same mtimes, misses after a touch', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'sc-'));
  const f = path.join(dir, 'corpus.json'); fs.writeFileSync(f, '[]');
  const c = createScopeCache(); let builds = 0;
  const build = (push) => { builds++; push({ key: 'ABC-1' }); push({ key: 'ABC-2' }); };
  const k = scopeKey('acct', ['XYZ', 'ABC']);
  const e1 = c.get(k, [f], build); const e2 = c.get(k, [f], build);
  assert.equal(builds, 1); assert.equal(e1, e2); assert.equal(e1.value.length, 2);
  let memoRuns = 0; c.memo(e1, 'agg', () => ++memoRuns); c.memo(e1, 'agg', () => ++memoRuns);
  assert.equal(memoRuns, 1);
  const t = new Date(Date.now() + 5000); fs.utimesSync(f, t, t);
  const e3 = c.get(k, [f], build);
  assert.equal(builds, 2); assert.notEqual(e3, e1);
});

test('LRU eviction and invalidate(prefix)', () => {
  const c = createScopeCache({ maxEntries: 2 });
  c.get('a::1', [], () => 1); c.get('b::1', [], () => 2); c.get('a::1', [], () => 1); c.get('c::1', [], () => 3);
  assert.deepEqual(c.keys(), ['a::1', 'c::1']);
  assert.equal(c.invalidate('a::'), 1); assert.equal(c.size(), 1);
  assert.equal(scopeKey('x', ['B', 'A']), scopeKey('x', ['A', 'B']));
});

test('getAsync: one in-flight rebuild per key, stale-while-revalidate for concurrent callers', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'sc-'));
  const f = path.join(dir, 'corpus.json'); fs.writeFileSync(f, '[]');
  const c = createScopeCache();
  let builds = 0;
  let release;
  const build = async () => { builds++; await new Promise((r) => { release = r; }); return { v: builds }; };
  const k = scopeKey('acct', ['ABC']);
  const p1 = c.getAsync(k, [f], build, { swr: true });
  await new Promise((r) => setImmediate(r));
  const p2 = c.getAsync(k, [f], build, { swr: true });
  release();
  const [e1, e2] = await Promise.all([p1, p2]);
  assert.equal(builds, 1); assert.equal(e1, e2); assert.equal(e1.stale, false);
  // Touch → first caller rebuilds (awaits fresh); a concurrent one gets the stale entry at once.
  const t = new Date(Date.now() + 5000); fs.utimesSync(f, t, t);
  const p3 = c.getAsync(k, [f], build, { swr: true });
  await new Promise((r) => setTimeout(r, 20));
  const stale = await c.getAsync(k, [f], build, { swr: true });
  assert.equal(stale, e1); assert.equal(stale.stale, true); assert.ok(c.inflight(k));
  release();
  const fresh = await p3;
  assert.equal(builds, 2); assert.equal(fresh.value.v, 2); assert.equal(fresh.stale, false);
  assert.equal(c.stats.stale_served, 1);
  assert.equal(c.entryOf(fresh.value), fresh);
});

test('default LRU holds 4 scopes; memo keyed by day-bucketed windows', async () => {
  const c = createScopeCache();
  for (const k of ['a', 'b', 'c', 'd', 'e']) await c.getAsync(k, [], async () => ({ k }));
  assert.deepEqual(c.keys(), ['b', 'c', 'd', 'e']);
  const { windowKey } = require('../lib/scopecache.js');
  const day = 86400000;
  assert.equal(windowKey(10 * day + 5, 20 * day + 100), windowKey(10 * day + 999, 20 * day + 5000));
  assert.notEqual(windowKey(10 * day, 20 * day), windowKey(10 * day, 21 * day));
  const e = c.get('e', [], () => 1);
  let runs = 0;
  c.memo(e, `met:${windowKey(0, 20 * day + 1)}`, () => ++runs);
  c.memo(e, `met:${windowKey(0, 20 * day + 9)}`, () => ++runs);
  assert.equal(runs, 1);
});

test('store: writeJsonAtomicAsync round-trips and concurrent writers never collide', async () => {
  const store = require('../lib/store.js');
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'st-'));
  const f = path.join(dir, 'sub', 'x.json');
  await Promise.all(Array.from({ length: 10 }, (_, i) => store.writeJsonAtomicAsync(f, { i })));
  const got = await store.readJsonAsync(f, null);
  assert.ok(got && Number.isInteger(got.i));
  assert.deepEqual(fs.readdirSync(path.dirname(f)), ['x.json'], 'no tmp files left');
  assert.equal(await store.readJsonAsync(path.join(dir, 'missing.json'), 'fb'), 'fb');
});
