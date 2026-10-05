// Pull-request ingestion THROUGH the Otto daemon (never straight to the forge):
// the daemon owns the git account credentials and provider quirks; we only
// pace our calls (lib/pacer.js) and keep an incremental per-repo cache.
//
// Cache: DATA_DIR/data/prs/<repoId>.json
//   { schema, cursor: {updated_on_max, page, synced_at}, prs: {[number]: Pr} }
// Detail (PR + commits + diff summary) is fetched only for PRs that are new or
// whose `updated_at` changed. The list walk stops once a whole page is older
// than the cursor (the daemon lists most-recently-updated first); `cursor.page`
// records the last finished page so an interrupted walk resumes there.
'use strict';

const fs = require('fs');
const path = require('path');

const SCHEMA = 1;
const KEY_RE = /[A-Z][A-Z0-9]+-\d+/g;
const PER_PAGE = 50;
const DAY_MS = 86400000;

// ---------- persistence ----------

function cacheFile(dataDir, repoId) {
  return path.join(dataDir, 'data', 'prs', `${String(repoId).replace(/[^a-zA-Z0-9_-]/g, '_')}.json`);
}

function loadCache(dataDir, repoId) {
  try {
    const obj = JSON.parse(fs.readFileSync(cacheFile(dataDir, repoId), 'utf8'));
    if (obj && obj.schema === SCHEMA && obj.prs && typeof obj.prs === 'object') {
      return { cursor: obj.cursor || {}, prs: obj.prs };
    }
  } catch { /* missing/corrupt → fresh */ }
  return { cursor: {}, prs: {} };
}

function saveCache(dataDir, repoId, cache) {
  const file = cacheFile(dataDir, repoId);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.tmp-${process.pid}`;
  fs.writeFileSync(tmp, JSON.stringify({ schema: SCHEMA, cursor: cache.cursor, prs: cache.prs }));
  fs.renameSync(tmp, file);
}

// ---------- normalization ----------

function extractKeys(...texts) {
  const out = new Set();
  for (const t of texts) for (const m of String(t || '').match(KEY_RE) || []) out.add(m);
  return [...out];
}

const iso = (v) => {
  if (!v) return null;
  const t = Date.parse(v);
  return Number.isFinite(t) ? new Date(t).toISOString() : null;
};
const minIso = (xs) => xs.filter(Boolean).sort()[0] || null;
const maxIso = (xs) => xs.filter(Boolean).sort().slice(-1)[0] || null;

function flattenComments(list, out = []) {
  for (const c of list || []) {
    out.push(c);
    flattenComments(c.replies, out);
  }
  return out;
}

/** Build the normalized Pr from list summary + PrDetail + commits + diff summary. */
function normalizePr(summary, detail, commits, diff) {
  const d = detail || {};
  const s = { ...summary, ...d };
  const author = s.author || '';
  const comments = flattenComments(d.comments);
  const reviewers = (d.reviewers || []).map((r) => ({
    name: r.name,
    approved: !!r.approved,
    reviewed_at: iso(r.reviewed_at),
  }));
  const approvals = reviewers.filter((r) => r.approved && r.name !== author).map((r) => r.reviewed_at);
  const nonAuthorComments = comments.filter((c) => c.author && c.author !== author).map((c) => iso(c.created_at));
  const nonAuthorReviews = reviewers.filter((r) => r.name !== author).map((r) => r.reviewed_at);
  const commitDates = (commits || []).map((c) => iso(c.date));
  const state = String(s.state || '').toLowerCase();
  const merged = state === 'merged';
  // PrSummary has no created/merged timestamps on every provider: prefer the
  // explicit fields, fall back to the earliest commit / last update (flagged).
  const openedAt = iso(s.created_at) || iso(s.opened_at) || minIso(commitDates);
  const mergedExplicit = iso(s.merged_at) || iso(s.closed_at);
  const files = diff && Array.isArray(diff.files) ? diff.files : null;
  const sum = (k) => (files ? files.reduce((a, f) => a + (Number(f[k]) || 0), 0) : null);
  return {
    number: s.number,
    title: s.title || '',
    source_branch: s.source_branch || '',
    target_branch: s.target_branch || '',
    author,
    url: s.url || null,
    state,
    updated_at: iso(s.updated_at),
    opened_at: openedAt,
    opened_at_source: iso(s.created_at) || iso(s.opened_at) ? 'provider' : openedAt ? 'first_commit' : null,
    first_review_at: minIso([...nonAuthorComments, ...nonAuthorReviews, ...approvals]),
    first_approval_at: minIso(approvals),
    last_approval_at: maxIso(approvals),
    merged_at: merged ? mergedExplicit || iso(s.updated_at) : null,
    merged_at_source: merged ? (mergedExplicit ? 'provider' : 'updated_at') : null,
    merge_commit: s.merge_commit_sha || (s.merge_commit && (s.merge_commit.hash || s.merge_commit.sha || s.merge_commit)) || null,
    head_sha: s.head_sha || null,
    commit_shas: (commits || []).map((c) => c.sha).filter(Boolean),
    additions: diff ? Number(diff.total_added ?? sum('added')) || 0 : null,
    deletions: diff ? Number(diff.total_deleted ?? sum('deleted')) || 0 : null,
    files: files ? files.length : null,
    comment_count: comments.length,
    reviewer_comment_count: nonAuthorComments.length,
    reviewers,
    keys: extractKeys(s.source_branch, s.title),
  };
}

// ---------- pure metrics ----------

/** Weekday (Mon–Fri, UTC) time between two instants, in fractional days. */
function businessDaysBetween(a, b) {
  let t = Date.parse(a);
  const end = Date.parse(b);
  if (!Number.isFinite(t) || !Number.isFinite(end) || end <= t) return 0;
  let ms = 0;
  while (t < end) {
    const dayEnd = (Math.floor(t / DAY_MS) + 1) * DAY_MS;
    const stop = Math.min(dayEnd, end);
    const dow = new Date(t).getUTCDay();
    if (dow !== 0 && dow !== 6) ms += stop - t;
    t = stop;
  }
  return ms / DAY_MS;
}

const r2 = (x) => Math.round(x * 100) / 100;

function sizeBucket(n) {
  if (n < 50) return '<50';
  if (n < 200) return '<200';
  if (n < 400) return '<400';
  return '>=400';
}

/**
 * pickup  = opened → first non-author review/comment/approval
 * review  = first review → last approval (merge when never approved)
 * merge_lag = last approval → merge
 * Each is null when either end is unknown.
 */
function derivePrMetrics(pr, businessDaysFn = businessDaysBetween) {
  const span = (a, b) => (a && b ? r2(businessDaysFn(a, b)) : null);
  const firstReview = pr.first_review_at || pr.first_approval_at;
  const reviewEnd = pr.last_approval_at || pr.merged_at;
  const size = pr.additions == null && pr.deletions == null ? null : (pr.additions || 0) + (pr.deletions || 0);
  const reviewerComments = pr.reviewer_comment_count ?? pr.comment_count ?? 0;
  return {
    pickup_days: span(pr.opened_at, firstReview),
    review_days: span(firstReview, reviewEnd),
    merge_lag_days: span(pr.last_approval_at, pr.merged_at),
    size,
    size_bucket: size == null ? null : sizeBucket(size),
    review_depth: size ? r2((reviewerComments / size) * 100) : null,
    unreviewed: !pr.first_review_at && !pr.first_approval_at,
  };
}

function pct(sorted, p) {
  if (!sorted.length) return null;
  const i = (sorted.length - 1) * p;
  const lo = Math.floor(i);
  const hi = Math.ceil(i);
  return r2(sorted[lo] + (sorted[hi] - sorted[lo]) * (i - lo));
}

/** p50/p75 per numeric metric over merged PRs; `{value:null}` when no data. */
function prFlowSummary(prs, businessDaysFn = businessDaysBetween) {
  const merged = (prs || []).filter((p) => p && p.merged_at);
  const ms = merged.map((p) => derivePrMetrics(p, businessDaysFn));
  const out = {};
  for (const k of ['pickup_days', 'review_days', 'merge_lag_days', 'size', 'review_depth']) {
    const xs = ms.map((m) => m[k]).filter((v) => v != null && Number.isFinite(v)).sort((a, b) => a - b);
    out[k] = xs.length ? { p50: pct(xs, 0.5), p75: pct(xs, 0.75), n: xs.length } : { value: null, n: 0 };
  }
  const buckets = { '<50': 0, '<200': 0, '<400': 0, '>=400': 0 };
  for (const m of ms) if (m.size_bucket) buckets[m.size_bucket]++;
  out.size_buckets = buckets;
  out.unreviewed = merged.length
    ? { count: ms.filter((m) => m.unreviewed).length, share: r2(ms.filter((m) => m.unreviewed).length / merged.length), n: merged.length }
    : { value: null, n: 0 };
  return out;
}

// ---------- daemon client ----------

class DaemonError extends Error {
  constructor(status, url, body) {
    super(`daemon ${status} for ${url}${body ? `: ${String(body).slice(0, 200)}` : ''}`);
    this.status = status;
  }
}

function createPrClient({ baseUrl, token, fetchImpl, pacer, dataDir, onProgress, withDiffstat = true } = {}) {
  if (!baseUrl) throw new Error('createPrClient: baseUrl required');
  if (!pacer) throw new Error('createPrClient: pacer required');
  if (!dataDir) throw new Error('createPrClient: dataDir required');
  const doFetch = fetchImpl || globalThis.fetch;
  const base = baseUrl.replace(/\/+$/, '');
  const progress = onProgress || (() => {});

  async function getJson(route) {
    const url = `${base}${route}`;
    const res = await pacer.schedule(() =>
      doFetch(url, { headers: { accept: 'application/json', ...(token ? { authorization: `Bearer ${token}` } : {}) } }),
    );
    if (!res || res.status < 200 || res.status >= 300) {
      let body = '';
      try { body = await res.text(); } catch { /* ignore */ }
      throw new DaemonError(res ? res.status : 0, route, body);
    }
    return res.json();
  }

  /** [{id, name, path, remote_url}] from GET /repos (array or {items}). */
  async function listRepos() {
    const r = await getJson('/repos');
    const items = Array.isArray(r) ? r : r.items || r.repos || [];
    return items.map((x) => ({ id: x.id, name: x.name, path: x.path, remote_url: x.remote_url || null }));
  }

  /** Map local repo paths → daemon repo ids (realpath-tolerant). */
  async function mapRepoPaths(paths) {
    const norm = (p) => {
      const clean = String(p || '').replace(/\/+$/, '');
      try { return fs.realpathSync(clean); } catch { return clean; }
    };
    const repos = await listRepos();
    const byPath = new Map(repos.map((r) => [norm(r.path), r.id]));
    const out = {};
    for (const p of paths || []) out[p] = byPath.get(norm(p)) || null;
    return out;
  }

  async function fetchDetail(repoId, n, summary) {
    const enc = encodeURIComponent(repoId);
    const detail = await getJson(`/repos/${enc}/prs/${n}`);
    const commits = await getJson(`/repos/${enc}/prs/${n}/commits`).catch(() => []);
    let diff = null;
    if (withDiffstat) diff = await getJson(`/repos/${enc}/prs/${n}/diff?summary=true`).catch(() => null);
    return normalizePr(summary, detail, Array.isArray(commits) ? commits : commits.items || [], diff);
  }

  /**
   * Incremental sync. `since` (ISO) bounds the first walk; later walks stop at
   * the stored cursor. Returns {repo, fetched, listed, pages, total, cursor}.
   */
  async function syncRepo(repoId, { since } = {}) {
    const cache = loadCache(dataDir, repoId);
    const stopAt = cache.cursor.updated_on_max || iso(since) || null;
    let page = cache.cursor.page && cache.cursor.in_progress ? cache.cursor.page + 1 : 1;
    let maxSeen = cache.cursor.updated_on_max || null;
    let fetched = 0;
    let listed = 0;
    let pages = 0;
    const enc = encodeURIComponent(repoId);
    for (;;) {
      const resp = await getJson(`/repos/${enc}/prs?state=all&page=${page}&per_page=${PER_PAGE}`);
      const items = Array.isArray(resp) ? resp : resp.items || [];
      pages++;
      listed += items.length;
      let anyNewer = false;
      for (const it of items) {
        const upd = iso(it.updated_at);
        if (upd && (!maxSeen || upd > maxSeen)) maxSeen = upd;
        if (stopAt && upd && upd <= stopAt) continue; // unchanged since the cursor / before `since`
        anyNewer = true;
        const prev = cache.prs[it.number];
        if (prev && prev.updated_at === upd) continue;
        cache.prs[it.number] = await fetchDetail(repoId, it.number, it);
        fetched++;
        progress({ repo: repoId, page, fetched, next_call_eta_ms: pacer.nextCallEtaMs(), backoff_ms: pacer.stats.last_backoff_ms });
      }
      cache.cursor = { ...cache.cursor, page, in_progress: true };
      saveCache(dataDir, repoId, cache);
      progress({ repo: repoId, page, fetched, next_call_eta_ms: pacer.nextCallEtaMs(), backoff_ms: pacer.stats.last_backoff_ms });
      const hasMore = Array.isArray(resp) ? items.length === PER_PAGE : !!resp.has_more;
      if (!hasMore || !items.length || (stopAt && !anyNewer)) break;
      page++;
    }
    cache.cursor = { updated_on_max: maxSeen, page: null, in_progress: false, synced_at: new Date().toISOString() };
    saveCache(dataDir, repoId, cache);
    return { repo: repoId, fetched, listed, pages, total: Object.keys(cache.prs).length, cursor: cache.cursor };
  }

  /** Freshness per cached repo (for /prs/status). */
  function status(repoIds) {
    return (repoIds || []).map((id) => {
      const c = loadCache(dataDir, id);
      return {
        repo: id,
        synced_at: c.cursor.synced_at || null,
        updated_on_max: c.cursor.updated_on_max || null,
        pending: !c.cursor.synced_at || !!c.cursor.in_progress,
        prs: Object.keys(c.prs).length,
      };
    });
  }

  return { listRepos, mapRepoPaths, syncRepo, status, loadPrs: (id) => Object.values(loadCache(dataDir, id).prs) };
}

module.exports = {
  createPrClient,
  normalizePr,
  derivePrMetrics,
  prFlowSummary,
  businessDaysBetween,
  extractKeys,
  loadCache,
  cacheFile,
  DaemonError,
};
