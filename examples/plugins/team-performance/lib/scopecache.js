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

async function fingerprintAsync(deps) {
  const ms = await Promise.all((deps || []).map((p) => fs.promises.stat(p).then((st) => st.mtimeMs, () => -1)));
  return (deps || []).map((p, i) => `${p}@${ms[i]}`).join('|');
}

/** Day-bucketed window key: memo keys stay stable within a day ("now" moves every ms). */
function windowKey(since, until, bucketMs = 86400000) {
  const b = (t) => (t ? Math.floor(Number(t) / bucketMs) : 0);
  return `${b(since)}-${b(until)}`;
}

function scopeKey(account, projects) {
  return `${account || ''}::${[...(projects || [])].map(String).sort().join(',')}`;
}

function createScopeCache({ maxEntries = 4 } = {}) {
  const map = new Map(); // key -> {fp, value, memos, builtAt}; insertion order = LRU order
  const stats = { hits: 0, misses: 0, stale_served: 0, coalesced: 0 };
  const inflight = new Map(); // key -> Promise<entry> (one rebuild per key at a time)

  function install(key, fp, out, records) {
    const entry = { key, fp, value: out === undefined ? records : out, records, memos: new Map(), builtAt: Date.now(), stale: false };
    map.delete(key);
    map.set(key, entry);
    while (map.size > maxEntries) map.delete(map.keys().next().value);
    return entry;
  }

  /**
   * Async get. A changed fingerprint starts ONE rebuild per key; concurrent
   * callers share its promise. With `swr`, callers arriving WHILE a rebuild
   * is already running get the previous entry at once (entry.stale = true)
   * instead of waiting — the caller that triggered the rebuild (usually the
   * one that just wrote) still awaits fresh data, so writes read back.
   */
  async function getAsync(key, deps, build, { swr = false } = {}) {
    const fp = await fingerprintAsync(deps);
    const cur = map.get(key);
    let p = inflight.get(key);
    if (!p && cur && cur.fp === fp) {
      map.delete(key); map.set(key, cur);
      stats.hits++;
      return cur;
    }
    if (p) {
      stats.coalesced++;
      if (swr && cur) {
        stats.stale_served++;
        cur.stale = true;
        return cur;
      }
      return p;
    }
    stats.misses++;
    p = (async () => {
      const records = [];
      const out = await build((r) => { records.push(r); }, records);
      return install(key, fp, out, records);
    })();
    inflight.set(key, p);
    p.then(() => inflight.delete(key), () => inflight.delete(key));
    return p;
  }

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
    return install(key, fp, out, records);
  }

  /** Per-entry memo; `name` should include every argument (e.g. a windowKey). Capped at 64 per entry. */
  function memo(entry, name, fn) {
    if (!entry || !entry.memos) return fn(entry && entry.value);
    if (!entry.memos.has(name)) {
      if (entry.memos.size >= 64) entry.memos.delete(entry.memos.keys().next().value);
      entry.memos.set(name, fn(entry.value));
    }
    return entry.memos.get(name);
  }

  /** The cached entry whose value is `value` (views hold the scope, not the entry). */
  function entryOf(value) {
    for (const e of map.values()) if (e.value === value) return e;
    return null;
  }

  function invalidate(prefix = '') {
    let n = 0;
    for (const k of [...map.keys()]) if (k.startsWith(prefix)) { map.delete(k); n++; }
    return n;
  }

  return { get, getAsync, memo, entryOf, inflight: (k) => inflight.has(k), invalidate, stats, size: () => map.size, keys: () => [...map.keys()] };
}

module.exports = { createScopeCache, scopeKey, fingerprint, fingerprintAsync, windowKey };
