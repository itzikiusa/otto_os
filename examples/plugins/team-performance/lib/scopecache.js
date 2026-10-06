// Scope cache: an LRU of derived scopes keyed by account+projects AND the
// mtimes of every dependency file (corpus, overrides, estimates, rework,
// people, config). Touch any dependency and the next get() rebuilds. Each
// entry also memoizes derived aggregates via memo(entry, name, fn) so the
// expensive passes run once per data version, not once per request.
'use strict';
const fs = require('fs');
const zlib = require('zlib');

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

// ── Parsed-file cache ──────────────────────────────────────────────────────
// Scope entries are keyed by EVERY dependency's mtime, so an overrides-only
// edit rebuilds the scope — but the (large) corpus file did not change. This
// cache keeps parsed JSON keyed by path + mtime + size and is shared across
// scope entries, so the rebuild re-reads only what actually changed. Bounded
// by the summed on-disk byte size (LRU eviction); a file larger than the cap
// is parsed but never retained. Callers must treat returned objects as
// READ-ONLY (they are shared).

function createFileCache({ maxBytes = 256 * 1024 * 1024 } = {}) {
  const map = new Map(); // path -> {sig, bytes, value}; insertion order = LRU
  const stats = { hits: 0, misses: 0, evictions: 0 };
  let bytes = 0;

  const sigOf = (st) => `${st.mtimeMs}:${st.size}`;

  function lookup(file, st) {
    const cur = map.get(file);
    if (cur && cur.sig === sigOf(st)) {
      map.delete(file); map.set(file, cur);
      stats.hits++;
      return cur;
    }
    return null;
  }

  function store(file, st, value) {
    drop(file);
    stats.misses++;
    if (st.size > maxBytes) return value;
    map.set(file, { sig: sigOf(st), bytes: st.size, value });
    bytes += st.size;
    while (bytes > maxBytes && map.size) {
      const k = map.keys().next().value;
      drop(k);
      stats.evictions++;
    }
    return value;
  }

  function drop(file) {
    const cur = map.get(file);
    if (!cur) return;
    bytes -= cur.bytes;
    map.delete(file);
  }

  /** Parsed JSON of `file`, or `fallback` when missing/unparsable (failures are not cached). */
  function readJson(file, fallback = null) {
    let st;
    try { st = fs.statSync(file); } catch { drop(file); return fallback; }
    const hit = lookup(file, st);
    if (hit) return hit.value;
    try { return store(file, st, JSON.parse(fs.readFileSync(file, 'utf8'))); } catch { return fallback; }
  }

  async function readJsonAsync(file, fallback = null) {
    let st;
    try { st = await fs.promises.stat(file); } catch { drop(file); return fallback; }
    const hit = lookup(file, st);
    if (hit) return hit.value;
    try { return store(file, st, JSON.parse(await fs.promises.readFile(file, 'utf8'))); } catch { return fallback; }
  }

  return { readJson, readJsonAsync, invalidate: drop, clear: () => { map.clear(); bytes = 0; }, stats, size: () => map.size, bytes: () => bytes };
}

/** Process-wide instance shared by every scope build. */
const fileCache = createFileCache();

// ── View memo ──────────────────────────────────────────────────────────────
// memoizeView(scope, key, fn) memoizes a derived view payload ON the scope
// object itself (WeakMap): a rebuilt scope is a new object, so stale views die
// with it — no separate invalidation. `key` must encode every argument (use
// windowKey for time windows). An async fn is memoized as its promise; a
// rejection is evicted so the next call retries.
//
// With { body: true } the result is { value, json, etag, gzip() } — `json` the
// serialized Buffer, `gzip()` the lazily compressed Buffer, both computed once
// per (scope, key) so repeat requests skip JSON.stringify and compression.

const viewMemos = new WeakMap(); // scope -> Map(key -> result)
const MAX_VIEWS_PER_SCOPE = 64;

function bodyOf(value) {
  const json = Buffer.from(JSON.stringify(value));
  let gz = null;
  return {
    value,
    json,
    etag: `W/"${json.length.toString(36)}-${require('crypto').createHash('sha1').update(json).digest('base64url').slice(0, 16)}"`,
    gzip() {
      if (!gz) gz = zlib.gzipSync(json, { level: 6 });
      return gz;
    },
  };
}

function memoizeView(scope, key, fn, { body = false } = {}) {
  if (!scope || (typeof scope !== 'object' && typeof scope !== 'function')) {
    const v = fn(scope);
    return body ? (v && typeof v.then === 'function' ? v.then(bodyOf) : bodyOf(v)) : v;
  }
  let m = viewMemos.get(scope);
  if (!m) { m = new Map(); viewMemos.set(scope, m); }
  const k = `${body ? 'b' : 'v'}:${key}`;
  if (m.has(k)) {
    const hit = m.get(k);
    m.delete(k); m.set(k, hit);
    return hit;
  }
  let out = fn(scope);
  if (out && typeof out.then === 'function') {
    out = out.then((v) => (body ? bodyOf(v) : v));
    out.catch(() => { if (m.get(k) === out) m.delete(k); });
  } else if (body) {
    out = bodyOf(out);
  }
  if (m.size >= MAX_VIEWS_PER_SCOPE) m.delete(m.keys().next().value);
  m.set(k, out);
  return out;
}

/** True when the request's Accept-Encoding allows gzip (q=0 excluded). */
function acceptsGzip(acceptEncoding) {
  return String(acceptEncoding || '')
    .split(',')
    .map((s) => s.trim().toLowerCase())
    .some((s) => /^(gzip|\*)(;|$)/.test(s) && !/;\s*q=0(\.0+)?\s*$/.test(s));
}

module.exports = { createScopeCache, scopeKey, fingerprint, fingerprintAsync, windowKey, createFileCache, fileCache, memoizeView, acceptsGzip };
