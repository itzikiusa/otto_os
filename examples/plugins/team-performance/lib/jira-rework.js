// Jira-detected rework: decides whether a ticket is really a redo of an
// earlier ticket (and which one), from Jira evidence plus git blame pairs.
//
// Signals (per candidate origin ticket):
//   link               "is caused by" / "caused by" / "fixes" / "is a fix for"
//                      (strong), or "relates to" (weak) — read from this
//                      ticket's links AND the reverse ("causes" / "is fixed by"
//                      on the origin pointing at this ticket)
//   title_ref          the summary names another known ticket key
//   title_keyword      /follow-up|fix for|missing|regression|forgot/ in the summary
//   bug_after_delivery a Bug created within bug_window_days after the origin's
//                      deploy whose commits rewrote the origin's code (blame)
//   blame              this ticket's commits rewrote the origin's recent code
//   reopened           the ticket itself was reopened after done (self rework)
//
// A key-shaped token whose project is not in the corpus ("JDK-17", "UTF-8") is
// a version/standard mention, never a reference. A rework ticket's estimate is
// NOT new delivered scope (scope_excluded) and its time is charged back to the
// origin's rework_in. Pure — no I/O.
'use strict';
const A = require('./analytics.js');

const DAY_MS = 86400000;
const KEY_RE = /\b([A-Z][A-Z0-9]+)-(\d+)\b/g;
const RE_KEYWORD = /follow.?up|fix(es)? for|missing|regression|forgot/i;
const RE_LINK_STRONG = /\b(is caused by|caused by|fixes|is a fix for|fix for|is a rework of|reworks)\b/i;
const RE_LINK_STRONG_REVERSE = /\b(causes|is fixed by|is reworked by)\b/i;
const RE_LINK_WEAK = /\brelates?\b/i;
const WEIGHT = { link: 3, bug_after_delivery: 3, title_ref: 2, link_weak: 1, blame: 1, title_keyword: 1 };
const round2 = (v) => Math.round(v * 100) / 100;

/**
 * Normalize Jira `issuelinks` (raw API shape) or already-normalized links to
 * [{rel, key}] where `rel` is the relation FROM this ticket's perspective
 * ("ABC-2 <rel> ABC-1", e.g. "is caused by").
 */
function normalizeLinks(links) {
  const out = [];
  for (const l of Array.isArray(links) ? links : []) {
    if (!l) continue;
    if (typeof l.rel === 'string' && l.key) { out.push({ rel: l.rel, key: l.key }); continue; }
    const t = l.type || {};
    if (l.outwardIssue && l.outwardIssue.key) out.push({ rel: t.outward || t.name || '', key: l.outwardIssue.key });
    if (l.inwardIssue && l.inwardIssue.key) out.push({ rel: t.inward || t.name || '', key: l.inwardIssue.key });
  }
  return out;
}

const linksOf = (r) => normalizeLinks(r.links || r.issuelinks);
const projectOf = (k) => String(k).split('-')[0];

/** key → [{rel, key}] of links on OTHER tickets pointing at it (reverse view). */
function reverseLinks(records) {
  const m = new Map();
  for (const r of records) {
    for (const l of linksOf(r)) {
      if (!m.has(l.key)) m.set(l.key, []);
      m.get(l.key).push({ rel: l.rel, key: r.key });
    }
  }
  return m;
}

const hasPair = (pairs, b, a) => {
  if (!pairs) return false;
  if (pairs instanceof Set) return pairs.has(`${b}>${a}`);
  return Boolean(pairs[`${b}>${a}`]);
};
const pairOrigins = (pairs, b) => {
  if (!pairs) return [];
  const ks = pairs instanceof Set ? [...pairs] : Object.keys(pairs);
  return ks.filter((k) => k.startsWith(`${b}>`)).map((k) => k.slice(b.length + 1));
};
/** B -> [A] index of blame pairs (avoids a full scan per record). */
function indexPairs(pairs) {
  if (!pairs) return null;
  const m = new Map();
  for (const k of pairs instanceof Set ? pairs : Object.keys(pairs)) {
    const i = k.indexOf('>');
    if (i < 0) continue;
    const b = k.slice(0, i);
    if (!m.has(b)) m.set(b, []);
    m.get(b).push(k.slice(i + 1));
  }
  return m;
}
const deliveredAt = (r) => (r ? r.deployed_at ?? r.delivered_at ?? r.done_git_at ?? r.done_at ?? null : null);

/**
 * Classify one record. `byKey` is a Map (or object) key → record.
 * opts: { blamePairs: rework.json pairs object or Set("B>A"),
 *         bug_window_days = 30, reverse: Map from reverseLinks() }
 * → { is_rework, origin_key, confidence: 'high'|'medium'|'low'|null, signals: [] }
 */
function classifyRework(record, byKey, opts = {}) {
  const get = (k) => (byKey instanceof Map ? byKey.get(k) : byKey ? byKey[k] : undefined);
  let projects = opts.projects;
  if (!projects) {
    projects = new Set();
    for (const k of byKey instanceof Map ? byKey.keys() : Object.keys(byKey || {})) projects.add(projectOf(k));
  }
  const windowMs = (opts.bug_window_days ?? 30) * DAY_MS;
  const self = record.key;
  const skip = new Set([self, record.parent_key].filter(Boolean));
  const cands = new Map(); // origin key -> Set(signal)
  const add = (k, s) => {
    if (!k || skip.has(k)) return;
    if (!cands.has(k)) cands.set(k, new Set());
    cands.get(k).add(s);
  };

  for (const l of linksOf(record)) {
    if (RE_LINK_STRONG.test(l.rel)) add(l.key, 'link');
    else if (RE_LINK_WEAK.test(l.rel)) add(l.key, 'link_weak');
  }
  for (const l of (opts.reverse && opts.reverse.get(self)) || []) {
    if (RE_LINK_STRONG_REVERSE.test(l.rel)) add(l.key, 'link');
    else if (RE_LINK_WEAK.test(l.rel)) add(l.key, 'link_weak');
  }
  const summary = String(record.summary || '');
  for (const m of summary.matchAll(KEY_RE)) {
    const k = `${m[1]}-${m[2]}`;
    // Only keys of a project we track: "JDK-17" / "UTF-8" are versions.
    if (get(k) || projects.has(m[1])) add(k, 'title_ref');
  }
  const keyword = RE_KEYWORD.test(summary);
  for (const a of opts.pairsIdx ? opts.pairsIdx.get(self) || [] : pairOrigins(opts.blamePairs, self)) add(a, 'blame');
  if (/bug/i.test(record.type || '') && record.created != null) {
    for (const [k, sig] of cands) {
      if (!sig.has('blame')) continue;
      const at = deliveredAt(get(k));
      if (at != null && record.created >= at && record.created - at <= windowMs) sig.add('bug_after_delivery');
    }
  }

  let best = null;
  for (const [k, sig] of cands) {
    if (keyword) sig.add('title_keyword');
    let score = 0;
    for (const s of sig) score += WEIGHT[s] || 0;
    // Blame alone only means "touched recent code" — already charged by the
    // line-level charge-back; it never makes the whole ticket a redo.
    if (sig.size === 1 && sig.has('blame')) continue;
    if (!best || score > best.score || (score === best.score && hasPair(opts.blamePairs, self, k))) best = { k, sig, score };
  }

  const reopened = Array.isArray(record.flags) && record.flags.includes('reopened');
  if (!best) {
    if (reopened) return { is_rework: true, origin_key: self, confidence: 'medium', signals: ['reopened', ...(keyword ? ['title_keyword'] : [])] };
    if (keyword) return { is_rework: false, origin_key: null, confidence: 'low', signals: ['title_keyword'] };
    return { is_rework: false, origin_key: null, confidence: null, signals: [] };
  }
  const confidence = best.score >= 3 ? 'high' : best.score === 2 ? 'medium' : 'low';
  const signals = [...best.sig].map((s) => (s === 'link_weak' ? 'link' : s));
  if (reopened) signals.push('reopened');
  return { is_rework: confidence !== 'low', origin_key: best.k, confidence, signals: [...new Set(signals)] };
}

/**
 * Apply Jira rework to the corpus (returns a new array; inputs untouched):
 *  - the rework record gets rework_of / rework_confidence / rework_signals and
 *    scope_excluded = true (its estimate is not new delivered scope);
 *  - its remaining actual (net of any blame charge-back already applied) moves
 *    to the origin's rework_in and its own rework_out.
 * A reopened ticket is its own origin: flagged (rework_self) but not excluded.
 * opts: { blamePairs, bug_window_days, actualDays = analytics.actualDays }
 */
function applyRework(records, opts = {}) {
  const actual = opts.actualDays || A.actualDays;
  const out = records.map((r) => ({ ...r }));
  const byKey = new Map(out.map((r) => [r.key, r]));
  const reverse = reverseLinks(out);
  const projects = new Set([...byKey.keys()].map(projectOf));
  const pairsIdx = indexPairs(opts.blamePairs);
  const verdicts = out.map((r) => (r.rollup ? null : classifyRework(r, byKey, { ...opts, reverse, projects, pairsIdx })));
  for (let i = 0; i < out.length; i++) {
    const v = verdicts[i];
    if (!v || !v.is_rework) continue;
    const r = out[i];
    r.rework_signals = v.signals;
    r.rework_confidence = v.confidence;
    if (v.origin_key === r.key) { r.rework_self = true; continue; }
    const origin = byKey.get(v.origin_key);
    r.rework_of = v.origin_key;
    r.scope_excluded = true;
    if (!origin) continue;
    // Charge only what the blame pair did not already move for this ticket.
    const t = actual(r) || 0;
    if (!(t > 0)) continue;
    origin.rework_in = round2((origin.rework_in || 0) + t);
    r.rework_out = round2((r.rework_out || 0) + t);
  }
  return out;
}

/**
 * Rework rate over the given (already scoped) records:
 * rework days / (dev days + rework days), where rework days = Σ rework_out
 * (time spent redoing other tickets' work) and dev = the non-rework time
 * (Σ actual − Σ rework_in: an origin's actual already carries the rework
 * charged back to it, which must not be counted twice).
 * Roll-up sub-tasks are skipped (their time lives in the story).
 */
function reworkRate(records, opts = {}) {
  const actual = opts.actualDays || A.actualDays;
  let rework = 0;
  let dev = 0;
  for (const r of records) {
    if (r.rollup) continue;
    rework += r.rework_out || 0;
    dev += (actual(r) || 0) - (r.rework_in || 0);
  }
  dev = Math.max(0, dev);
  const total = dev + rework;
  return { rate: total > 0 ? round2(rework / total) : null, rework_days: round2(rework), dev_days: round2(dev) };
}

/** Delivered scope (Σ estimate) that skips scope_excluded rework tickets. */
function deliveredScope(records, estimateOf) {
  let s = 0;
  for (const r of records) if (!r.scope_excluded && !r.rollup) s += estimateOf(r) || 0;
  return round2(s);
}

module.exports = { classifyRework, applyRework, reworkRate, deliveredScope, normalizeLinks, reverseLinks };
