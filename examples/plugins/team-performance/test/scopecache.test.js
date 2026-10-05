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

// ── Parsed-file cache ──
const zlib = require('zlib');
const { createFileCache, memoizeView, acceptsGzip } = require('../lib/scopecache.js');

test('file cache: reuses parse while mtime unchanged, shared across scope rebuilds', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'fc-'));
  const corpus = path.join(dir, 'corpus.json'); fs.writeFileSync(corpus, JSON.stringify({ issues: { 'ABC-1': {} } }));
  const ov = path.join(dir, 'overrides.json'); fs.writeFileSync(ov, '{}');
  const fc = createFileCache();
  const sc = createScopeCache();
  const build = () => ({ corpus: fc.readJson(corpus), ov: fc.readJson(ov) });
  const e1 = sc.get('k', [corpus, ov], build);
  // overrides-only change: scope rebuilds, corpus is NOT re-parsed
  fs.writeFileSync(ov, '{"issues":{}}'); const t = new Date(Date.now() + 5000); fs.utimesSync(ov, t, t);
  const e2 = sc.get('k', [corpus, ov], build);
  assert.notEqual(e1, e2);
  assert.equal(e2.value.corpus, e1.value.corpus, 'same parsed corpus object');
  assert.deepEqual(e2.value.ov, { issues: {} });
  assert.equal(fc.stats.hits, 1);
  // corpus touched → re-parsed
  fs.writeFileSync(corpus, '{"issues":{}}'); fs.utimesSync(corpus, t, new Date(Date.now() + 9000));
  assert.notEqual(fc.readJson(corpus), e1.value.corpus);
  // missing / bad files → fallback, not cached
  assert.equal(fc.readJson(path.join(dir, 'nope.json'), 'fb'), 'fb');
  const bad = path.join(dir, 'bad.json'); fs.writeFileSync(bad, '{');
  assert.equal(fc.readJson(bad, null), null);
  fs.rmSync(dir, { recursive: true, force: true });
});

test('file cache: size cap evicts LRU and never retains oversize files', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'fc-'));
  const mk = (n, len) => { const f = path.join(dir, `${n}.json`); fs.writeFileSync(f, JSON.stringify('x'.repeat(len))); return f; };
  const a = mk('a', 40), b = mk('b', 40), big = mk('big', 500);
  const fc = createFileCache({ maxBytes: 100 });
  fc.readJson(a); fc.readJson(b);
  assert.equal(fc.size(), 2);
  const c = mk('c', 40); await fc.readJsonAsync(c);
  assert.equal(fc.size(), 2); assert.equal(fc.stats.evictions, 1);
  assert.ok(fc.bytes() <= 100);
  fc.readJson(big);
  assert.equal(fc.size(), 2, 'oversize file parsed but not kept');
  fs.rmSync(dir, { recursive: true, force: true });
});

test('memoizeView: once per scope+key, dies with the scope, gzip body cached', async () => {
  const scope = { records: [1, 2, 3] };
  let runs = 0;
  const fn = (s) => { runs++; return { total: s.records.length }; };
  assert.deepEqual(memoizeView(scope, 'overview:1-2', fn), { total: 3 });
  memoizeView(scope, 'overview:1-2', fn);
  assert.equal(runs, 1);
  memoizeView(scope, 'overview:3-4', fn);
  assert.equal(runs, 2, 'different key recomputes');
  memoizeView({ records: [] }, 'overview:1-2', fn);
  assert.equal(runs, 3, 'new scope object recomputes');

  const b1 = memoizeView(scope, 'assignee:X', fn, { body: true });
  const b2 = memoizeView(scope, 'assignee:X', fn, { body: true });
  assert.equal(b1, b2);
  assert.deepEqual(JSON.parse(b1.json), { total: 3 });
  assert.equal(b1.gzip(), b1.gzip(), 'gzip computed once');
  assert.deepEqual(JSON.parse(zlib.gunzipSync(b1.gzip())), { total: 3 });
  assert.match(b1.etag, /^W\/"/);

  // async fn: rejection is evicted and retried
  let n = 0;
  const af = async () => { n++; if (n === 1) throw new Error('boom'); return { ok: true }; };
  await assert.rejects(memoizeView(scope, 'a', af));
  await new Promise((r) => setImmediate(r));
  assert.deepEqual((await memoizeView(scope, 'a', af, {})), { ok: true });
  assert.equal((await memoizeView(scope, 'ab', af, { body: true })).value.ok, true);
});

test('acceptsGzip parses Accept-Encoding', () => {
  assert.equal(acceptsGzip('gzip, deflate, br'), true);
  assert.equal(acceptsGzip('br;q=1.0, gzip;q=0.8'), true);
  assert.equal(acceptsGzip('gzip;q=0'), false);
  assert.equal(acceptsGzip('*'), true);
  assert.equal(acceptsGzip(''), false);
  assert.equal(acceptsGzip(undefined), false);
});
