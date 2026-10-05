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

/**
 * Missing file → fresh cache. Unparseable / wrong-shape file → fresh cache
 * too, but flagged `recovered: true` and reported through `onWarn` so the
 * corruption is visible in logs instead of silently re-walking everything.
 */
function loadCache(dataDir, repoId, onWarn) {
  const file = cacheFile(dataDir, repoId);
  let raw;
  try {
    raw = fs.readFileSync(file, 'utf8');
  } catch {
    return { cursor: {}, prs: {}, meta: {} };
  }
  try {
    const obj = JSON.parse(raw);
    if (obj && obj.schema === SCHEMA && obj.prs && typeof obj.prs === 'object') {
      return { cursor: obj.cursor || {}, prs: obj.prs, meta: obj.meta || {} };
    }
    throw new Error(`unexpected cache shape (schema ${obj && obj.schema})`);
  } catch (e) {
    if (onWarn) onWarn({ repo: repoId, file, error: String(e.message || e) });
    return { cursor: {}, prs: {}, meta: {}, recovered: true };
  }
}

function saveCache(dataDir, repoId, cache) {
  const file = cacheFile(dataDir, repoId);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.tmp-${process.pid}`;
  fs.writeFileSync(tmp, JSON.stringify({ schema: SCHEMA, cursor: cache.cursor, prs: cache.prs, meta: cache.meta || {} }));
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
    id: r.id ?? r.account_id ?? r.uuid ?? null,
    approved: !!r.approved,
    reviewed_at: iso(r.reviewed_at),
  }));
  const approvals = reviewers.filter((r) => r.approved && r.name !== author).map((r) => r.reviewed_at);
  const nonAuthorComments = comments.filter((c) => c.author && c.author !== author).map((c) => iso(c.created_at));
  const nonAuthorReviews = reviewers.filter((r) => r.name !== author).map((r) => r.reviewed_at);
  const commitDates = (commits || []).map((c) => iso(c.date || c.committed_at || c.authored_at));
  const stateRaw = String(s.state || '').toLowerCase();
  const state = normalizeState(stateRaw, s);
  const merged = state === 'merged';
  const commentAuthors = {};
  for (const c of comments) if (c.author) commentAuthors[c.author] = (commentAuthors[c.author] || 0) + 1;
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
    state_raw: stateRaw,
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
    commit_dates: commitDates.filter(Boolean).sort(),
    first_commit_at: minIso(commitDates),
    last_commit_at: maxIso(commitDates),
    additions: diff ? Number(diff.total_added ?? sum('added')) || 0 : null,
    deletions: diff ? Number(diff.total_deleted ?? sum('deleted')) || 0 : null,
    files: files ? files.length : null,
    comment_count: comments.length,
    reviewer_comment_count: nonAuthorComments.length,
    comment_authors: commentAuthors,
    review_event_dates: [...nonAuthorComments, ...nonAuthorReviews].filter(Boolean).sort(),
    reviewers,
    reviewer_ids: reviewers.filter((r) => r.name !== author).map((r) => r.id ?? r.name).filter((x) => x != null),
    approvers: reviewers.filter((r) => r.approved && r.name !== author).map((r) => r.name),
    approvals_count: approvals.length,
    keys: extractKeys(s.source_branch, s.title),
  };
}

/** Provider states → merged | declined | open. */
function normalizeState(raw, s = {}) {
  if (raw === 'merged' || (s.merged === true)) return 'merged';
  if (['declined', 'closed', 'superseded', 'rejected'].includes(raw)) return 'declined';
  if (!raw && (s.merged_at)) return 'merged';
  return 'open';
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
  const commitDates = pr.commit_dates || [];
  const firstCommit = pr.first_commit_at || commitDates[0] || null;
  const postReview = firstReview ? commitDates.filter((t) => t > firstReview).length : 0;
  return {
    coding_days: firstCommit && pr.opened_at && firstCommit < pr.opened_at ? span(firstCommit, pr.opened_at) : firstCommit && pr.opened_at ? 0 : null,
    pr_cycle_days: span(pr.opened_at, pr.merged_at),
    comments_count: pr.comment_count ?? 0,
    reviewer_comments: reviewerComments,
    reviewers_n: new Set((pr.reviewers || []).filter((r) => r.name && r.name !== pr.author).map((r) => r.name)
      .concat(Object.keys(pr.comment_authors || {}).filter((a) => a !== pr.author))).size,
    post_review_commits: postReview,
    review_rounds: reviewRounds(pr.review_event_dates || (firstReview ? [firstReview] : []), commitDates),
    merged_without_approval: !!pr.merged_at && !pr.first_approval_at,
    pickup_days: span(pr.opened_at, firstReview),
    review_days: span(firstReview, reviewEnd),
    merge_lag_days: span(pr.last_approval_at, pr.merged_at),
    size,
    size_bucket: size == null ? null : sizeBucket(size),
    review_depth: size ? r2((reviewerComments / size) * 100) : null,
    unreviewed: !pr.first_review_at && !pr.first_approval_at,
  };
}

/**
 * A review round = a burst of review activity; a new round starts when the
 * author pushed commits after the previous review activity and reviewers came
 * back. 0 when nobody reviewed.
 */
function reviewRounds(reviewDates, commitDates) {
  const ev = [...reviewDates.map((t) => [t, 'r']), ...commitDates.map((t) => [t, 'c'])].sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
  let rounds = 0;
  let pushedSinceReview = true;
  for (const [, k] of ev) {
    if (k === 'c') { if (rounds) pushedSinceReview = true; continue; }
    if (pushedSinceReview) { rounds++; pushedSinceReview = false; }
  }
  return rounds;
}

function pct(sorted, p) {
  if (!sorted.length) return null;
  const i = (sorted.length - 1) * p;
  const lo = Math.floor(i);
  const hi = Math.ceil(i);
  return r2(sorted[lo] + (sorted[hi] - sorted[lo]) * (i - lo));
}

const dist = (vals) => {
  const xs = vals.filter((v) => v != null && Number.isFinite(v)).sort((a, b) => a - b);
  return xs.length ? { p50: pct(xs, 0.5), p75: pct(xs, 0.75), n: xs.length } : { value: null, n: 0 };
};

const FLOW_KEYS = ['pickup_days', 'review_days', 'merge_lag_days', 'coding_days', 'pr_cycle_days', 'size', 'review_depth',
  'comments_count', 'reviewers_n', 'post_review_commits', 'review_rounds'];

/**
 * Flow summary over a PR set (the caller scopes it to a period/team).
 * Timing distributions use MERGED PRs only; `by_person` counts every PR passed.
 * `opts.capacityDays` ({person: available working days, vacations already
 * removed}) adds per-capacity-day rates — never raw counts as productivity.
 * Everything is `{value:null, n:0}` when there is nothing to measure.
 */
function prFlowSummary(prs, businessDaysFn = businessDaysBetween, opts = {}) {
  if (businessDaysFn && typeof businessDaysFn === 'object') { opts = businessDaysFn; businessDaysFn = businessDaysBetween; }
  const fn = businessDaysFn || businessDaysBetween;
  const all = (prs || []).filter(Boolean);
  const merged = all.filter((p) => p.merged_at);
  const ms = merged.map((p) => derivePrMetrics(p, fn));
  const out = {};
  for (const k of FLOW_KEYS) out[k] = dist(ms.map((m) => m[k]));
  const buckets = { '<50': 0, '<200': 0, '<400': 0, '>=400': 0 };
  for (const m of ms) if (m.size_bucket) buckets[m.size_bucket]++;
  out.size_buckets = buckets;
  const share = (pred) => (merged.length
    ? { count: ms.filter(pred).length, share: r2(ms.filter(pred).length / merged.length), n: merged.length }
    : { value: null, n: 0 });
  out.unreviewed = share((m) => m.unreviewed);
  out.merged_without_approval = share((m) => m.merged_without_approval);
  out.reworked_after_review = share((m) => m.post_review_commits > 0);
  out.counts = {
    total: all.length,
    merged: merged.length,
    declined: all.filter((p) => p.state === 'declined').length,
    open: all.filter((p) => !p.merged_at && p.state !== 'declined').length,
  };
  out.by_person = prByPerson(all, opts.capacityDays);
  out.review_load = reviewLoad(out.by_person);
  return out;
}

function prByPerson(prs, capacityDays) {
  const people = {};
  const get = (n) => (people[n] ||= { authored: 0, merged: 0, reviewed_given: 0, comments_given: 0, approvals_given: 0 });
  for (const pr of prs) {
    if (pr.author) {
      const a = get(pr.author);
      a.authored++;
      if (pr.merged_at) a.merged++;
    }
    const reviewers = new Set();
    for (const r of pr.reviewers || []) {
      if (!r.name || r.name === pr.author) continue;
      reviewers.add(r.name);
      if (r.approved) get(r.name).approvals_given++;
    }
    for (const [name, n] of Object.entries(pr.comment_authors || {})) {
      if (name === pr.author) continue;
      reviewers.add(name);
      get(name).comments_given += n;
    }
    for (const name of reviewers) get(name).reviewed_given++;
  }
  if (capacityDays) {
    for (const [name, p] of Object.entries(people)) {
      const cap = Number(capacityDays[name]);
      p.capacity_days = Number.isFinite(cap) ? cap : null;
      p.per_capacity_day = cap > 0
        ? { authored: r2(p.authored / cap), reviewed_given: r2(p.reviewed_given / cap), comments_given: r2(p.comments_given / cap) }
        : null; // unknown/zero capacity → no rate, never divide-by-guess
    }
  }
  return people;
}

/** How evenly review work is spread: per-reviewer share + top-reviewer share. */
function reviewLoad(byPerson) {
  const rows = Object.entries(byPerson).filter(([, p]) => p.reviewed_given > 0)
    .map(([name, p]) => ({ name, reviews: p.reviewed_given })).sort((a, b) => b.reviews - a.reviews || a.name.localeCompare(b.name));
  const total = rows.reduce((a, r) => a + r.reviews, 0);
  if (!total) return { value: null, n: 0, reviewers: [] };
  for (const r of rows) r.share = r2(r.reviews / total);
  return { n: total, reviewers: rows, top_share: rows[0].share, reviewers_n: rows.length };
}

/**
 * Compact per-Jira-key PR digest for scope memory: numbers, timings, counts —
 * no titles, bodies, comment text or diffs. {[KEY]: summary}
 */
function prSummaryByKey(prs, businessDaysFn = businessDaysBetween) {
  const by = {};
  for (const pr of prs || []) {
    if (!pr) continue;
    const m = derivePrMetrics(pr, businessDaysFn);
    for (const key of pr.keys || []) {
      const e = (by[key] ||= { prs: [], authors: [], opened_first: null, merged_last: null, size: 0, comments: 0,
        post_review_commits: 0, unreviewed: 0, merged_without_approval: 0 });
      e.prs.push({ number: pr.number, state: pr.state, url: pr.url || null, author: pr.author || null,
        opened_at: pr.opened_at, merged_at: pr.merged_at, pickup_days: m.pickup_days, pr_cycle_days: m.pr_cycle_days,
        size: m.size, review_rounds: m.review_rounds });
      if (pr.author && !e.authors.includes(pr.author)) e.authors.push(pr.author);
      e.opened_first = minIso([e.opened_first, pr.opened_at]);
      e.merged_last = maxIso([e.merged_last, pr.merged_at]);
      e.size += m.size || 0;
      e.comments += m.comments_count;
      e.post_review_commits += m.post_review_commits;
      if (pr.merged_at && m.unreviewed) e.unreviewed++;
      if (m.merged_without_approval) e.merged_without_approval++;
    }
  }
  return by;
}

/**
 * First-class status for the PR ingest, so the UI can explain an empty panel.
 *   not_configured — no daemon API/token, or no repo mapped to a daemon repo
 *   never_run      — configured, but no repo has finished a sync or failed
 *   failing        — a repo's last attempt errored after its last good sync
 *   ok             — every mapped repo synced at least once (some may be pending)
 * `runtime` is the server's in-memory {error, repos:{[id]:{ok,error}}} state.
 */
function prIngestStatus({ dataDir, configured, repoIds, runtime = {}, unregistered = [] } = {}) {
  const ids = [...new Set(repoIds || [])];
  const cursorByRepo = {};
  let fetched = 0;
  const pending = [];
  const errors = [];
  let anySynced = false;
  for (const id of ids) {
    const c = dataDir ? loadCache(dataDir, id) : { cursor: {}, prs: {}, meta: {} };
    const n = Object.keys(c.prs).length;
    fetched += n;
    cursorByRepo[id] = { updated_on_max: c.cursor.updated_on_max || null, synced_at: c.cursor.synced_at || null,
      in_progress: !!c.cursor.in_progress, prs: n, recovered: !!c.recovered };
    if (c.cursor.synced_at) anySynced = true;
    if (!c.cursor.synced_at || c.cursor.in_progress) pending.push(id);
    const rt = runtime.repos && runtime.repos[id];
    const err = (rt && rt.ok === false && rt.error) || (c.meta.last_error && (!c.cursor.synced_at || c.meta.last_error_at >= c.cursor.synced_at) ? c.meta.last_error : null);
    if (err) errors.push({ repo: id, error: err });
  }
  const base = { cursor_by_repo: cursorByRepo, fetched, pending, unregistered };
  if (!configured) {
    return { state: 'not_configured', reason: runtime.error || 'Otto daemon API token not available to the plugin — PR data cannot be fetched.', last_error: null, ...base };
  }
  if (!ids.length) {
    return { state: 'not_configured', reason: unregistered.length
      ? `None of the scanned repositories is registered in Otto (${unregistered.length} unregistered) — add them in Git to enable PR metrics.`
      : 'No repositories mapped to Otto yet — run a scan to map local repos.', last_error: null, ...base };
  }
  if (errors.length || runtime.error) {
    const last = runtime.error || errors[errors.length - 1].error;
    return { state: 'failing', reason: `PR sync failed for ${errors.length || 'all'} repo(s); cached data is shown and the cursor is unchanged.`, last_error: last, errors, ...base };
  }
  if (!anySynced) return { state: 'never_run', reason: 'PR sync has not completed yet — it runs as part of a scan.', last_error: null, ...base };
  return { state: 'ok', reason: pending.length ? `${pending.length} repo(s) still syncing.` : null, last_error: null, ...base };
}

// ---------- daemon client ----------

class DaemonError extends Error {
  constructor(status, url, body) {
    super(`daemon ${status} for ${url}${body ? `: ${String(body).slice(0, 200)}` : ''}`);
    this.status = status;
  }
}

function createPrClient({ baseUrl, token, fetchImpl, pacer, dataDir, onProgress, onWarn, withDiffstat = true } = {}) {
  if (!baseUrl) throw new Error('createPrClient: baseUrl required');
  if (!pacer) throw new Error('createPrClient: pacer required');
  if (!dataDir) throw new Error('createPrClient: dataDir required');
  const doFetch = fetchImpl || globalThis.fetch;
  const base = baseUrl.replace(/\/+$/, '');
  const progress = onProgress || (() => {});
  const warn = onWarn || ((w) => console.warn(`[team-performance] PR cache for ${w.repo} unreadable, starting fresh: ${w.error}`));

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
    const cache = loadCache(dataDir, repoId, warn);
    const startCursor = { ...cache.cursor };
    try {
      return await walk(repoId, cache, since);
    } catch (e) {
      // Keep any details already fetched (they are keyed by updated_at so a
      // re-run skips them) but restore the cursor: the walk did not finish.
      cache.cursor = startCursor;
      cache.meta = { ...(cache.meta || {}), last_error: String(e.message || e).slice(0, 300), last_error_status: e.status || null,
        last_error_at: new Date().toISOString() };
      saveCache(dataDir, repoId, cache);
      throw e;
    }
  }

  async function walk(repoId, cache, since) {
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
      // Page progress is persisted only after the walk proves it can finish a
      // page; on failure syncRepo() restores the starting cursor.
      saveCache(dataDir, repoId, { ...cache, cursor: { ...cache.cursor, page, in_progress: true } });
      progress({ repo: repoId, page, fetched, next_call_eta_ms: pacer.nextCallEtaMs(), backoff_ms: pacer.stats.last_backoff_ms });
      const hasMore = Array.isArray(resp) ? items.length === PER_PAGE : !!resp.has_more;
      if (!hasMore || !items.length || (stopAt && !anyNewer)) break;
      page++;
    }
    cache.cursor = { updated_on_max: maxSeen, page: null, in_progress: false, synced_at: new Date().toISOString() };
    cache.meta = { ...(cache.meta || {}), last_error: null, last_ok_at: cache.cursor.synced_at };
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
  prSummaryByKey,
  prIngestStatus,
  normalizeState,
  reviewRounds,
  businessDaysBetween,
  extractKeys,
  loadCache,
  cacheFile,
  DaemonError,
};
