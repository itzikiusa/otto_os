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
