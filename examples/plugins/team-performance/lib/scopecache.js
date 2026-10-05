// Scope cache: an LRU of derived scopes keyed by account+projects AND the
// mtimes of every dependency file (corpus, overrides, estimates, rework,
// people, config). Touch any dependency and the next get() rebuilds. Each
// entry also memoizes derived aggregates via memo(entry, name, fn) so the
// expensive passes run once per data version, not once per request.
'use strict';
const fs = require('fs');

function mtimeOf(p) {
  try { return fs.statSync(p).mtimeMs; } catch { return -1; } // missing file is a valid (stable) state
}

function fingerprint(deps) {
  return (deps || []).map((p) => `${p}@${mtimeOf(p)}`).join('|');
}

function scopeKey(account, projects) {
  return `${account || ''}::${[...(projects || [])].map(String).sort().join(',')}`;
}

function createScopeCache({ maxEntries = 8 } = {}) {
  const map = new Map(); // key -> {fp, value, memos, builtAt}; insertion order = LRU order
  const stats = { hits: 0, misses: 0 };

  function get(key, deps, build) {
    const fp = fingerprint(deps);
    const cur = map.get(key);
    if (cur && cur.fp === fp) {
      map.delete(key); map.set(key, cur); // refresh recency
      stats.hits++;
      return cur;
    }
    stats.misses++;
    // build(push) may either return a value or fill records via push — one array, no concat.
    const records = [];
    const push = (r) => { records.push(r); };
    const out = build(push, records);
    const entry = { key, fp, value: out === undefined ? records : out, records, memos: new Map(), builtAt: Date.now() };
    map.delete(key);
    map.set(key, entry);
    while (map.size > maxEntries) map.delete(map.keys().next().value);
    return entry;
  }

  function memo(entry, name, fn) {
    if (!entry || !entry.memos) return fn();
    if (!entry.memos.has(name)) entry.memos.set(name, fn(entry.value));
    return entry.memos.get(name);
  }

  function invalidate(prefix = '') {
    let n = 0;
    for (const k of [...map.keys()]) if (k.startsWith(prefix)) { map.delete(k); n++; }
    return n;
  }

  return { get, memo, invalidate, stats, size: () => map.size, keys: () => [...map.keys()] };
}

module.exports = { createScopeCache, scopeKey, fingerprint };
