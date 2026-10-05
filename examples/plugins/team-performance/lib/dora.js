// DORA metrics — pure functions, no I/O.
//
// Inputs are what the scan already persists:
//   records: issue records ({key, type, created, first_commit_at, deployed_at,
//            deployed_tag?, rework_of?, origin?})
//   tags:    deploy tags from gitscan ({repo, name, ts, kind: 'hotfix'|'regular'})
//
// Definitions (documented so a number is never misread):
//   deploy_frequency    distinct (repo, UTC day) deploy events in the window,
//                       per week — several tags on one day in one repo = 1 deploy.
//   lead_time           business days first_commit_at → deployed_at, over
//                       records deployed inside the window (p50 / p75).
//   change_failure_rate failed deploys / deploys. A deploy FAILED when, within
//                       failure_window_days after it, the same repo got a
//                       hotfix-kind tag, or a Bug was created whose
//                       rework_of/origin points at a key that deploy shipped.
//   mttr                failure signal (bug created, or the hotfix tag itself)
//                       → next deploy tag after it (hours; p50).
// Every metric is {value:null} (never 0 / NaN) when its denominator is empty,
// and gets a guardrail when its sample is below min_n.
'use strict';

const { businessDays } = require('./analytics.js');

const DAY = 86400000;
const HOUR = 3600000;

// DORA (State of DevOps) performance bands. Lower bound inclusive for
// frequency; upper bound inclusive for the "smaller is better" metrics.
const DORA_BANDS = {
  // deploys per week: elite = on-demand (≥ daily ≈ 5/wk business days),
  // high = weekly…daily, medium = monthly…weekly, low = < monthly.
  deploy_frequency: [
    { band: 'elite', min: 5 },
    { band: 'high', min: 1 },
    { band: 'medium', min: 0.25 },
    { band: 'low', min: 0 },
  ],
  // lead time for changes, business days: elite < 1 day, high ≤ 1 week,
  // medium ≤ 1 month (~22 business days), low beyond.
  lead_time_days: [
    { band: 'elite', max: 1 },
    { band: 'high', max: 5 },
    { band: 'medium', max: 22 },
    { band: 'low', max: Infinity },
  ],
  // change failure rate (fraction): elite ≤ 5%, high ≤ 15%, medium ≤ 30%.
  change_failure_rate: [
    { band: 'elite', max: 0.05 },
    { band: 'high', max: 0.15 },
    { band: 'medium', max: 0.3 },
    { band: 'low', max: Infinity },
  ],
  // time to restore, hours: elite < 1h, high < 1 day, medium < 1 week.
  mttr_hours: [
    { band: 'elite', max: 1 },
    { band: 'high', max: 24 },
    { band: 'medium', max: 168 },
    { band: 'low', max: Infinity },
  ],
};

function bandFor(metric, value) {
  if (value === null || value === undefined || Number.isNaN(value)) return null;
  for (const b of DORA_BANDS[metric]) {
    if (b.min !== undefined ? value >= b.min : value <= b.max) return b.band;
  }
  return null;
}

const round2 = (x) => Math.round(x * 100) / 100;

/** Linear-interpolated percentile (q in 0..1) of a numeric array; null when empty. */
function percentile(values, q) {
  const v = values.filter((x) => typeof x === 'number' && Number.isFinite(x)).sort((a, b) => a - b);
  if (!v.length) return null;
  const pos = (v.length - 1) * q;
  const lo = Math.floor(pos);
  const hi = Math.ceil(pos);
  return round2(v[lo] + (v[hi] - v[lo]) * (pos - lo));
}

const inWindow = (t, w) => typeof t === 'number' && t >= w.start && t < w.end;
const isBug = (r) => /bug|defect|incident/i.test(String(r.type || ''));
const refsOf = (r) => {
  const out = [];
  for (const f of [r.rework_of, r.origin]) {
    if (Array.isArray(f)) out.push(...f);
    else if (f && typeof f === 'object' && f.key) out.push(f.key);
    else if (typeof f === 'string' && f) out.push(f);
  }
  return out;
};

/**
 * doraMetrics({records, tags, window:{start,end}, cfg:{failure_window_days, min_n}})
 * → {deploy_frequency, lead_time, change_failure_rate, mttr, guardrails, bands}
 */
function doraMetrics({ records = [], tags = [], window = {}, cfg = {} } = {}) {
  const failWinMs = (Number(cfg.failure_window_days) > 0 ? Number(cfg.failure_window_days) : 7) * DAY;
  const minN = Number(cfg.min_n) > 0 ? Number(cfg.min_n) : 5;
  const allTs = tags.map((t) => t.ts).filter(Number.isFinite);
  const w = {
    start: Number.isFinite(window.start) ? window.start : allTs.length ? Math.min(...allTs) : 0,
    end: Number.isFinite(window.end) ? window.end : allTs.length ? Math.max(...allTs) + 1 : 0,
  };
  const guardrails = [];
  const guard = (metric, n, what) => {
    if (n < minN) guardrails.push({ code: 'low_n', metric, reason: `${what}: n=${n} < ${minN}; treat as indicative only` });
  };

  // ---- deploy events: collapse same-day tags per repo ---------------------
  const sorted = [...tags].filter((t) => Number.isFinite(t.ts)).sort((a, b) => a.ts - b.ts);
  const deploys = new Map(); // "repo|day" -> {repo, day, ts, tags:[], kind}
  for (const t of sorted) {
    const day = Math.floor(t.ts / DAY);
    const id = `${t.repo || ''}|${day}`;
    let d = deploys.get(id);
    if (!d) {
      d = { id, repo: t.repo || null, ts: t.ts, tags: [], kinds: new Set() };
      deploys.set(id, d);
    }
    d.tags.push(t.name);
    d.kinds.add(t.kind || 'regular');
  }
  const deployList = [...deploys.values()];
  const windowDeploys = deployList.filter((d) => inWindow(d.ts, w));

  const weeks = (w.end - w.start) / (7 * DAY);
  const total = windowDeploys.length;
  const byRepo = {};
  for (const d of windowDeploys) byRepo[d.repo || 'unknown'] = (byRepo[d.repo || 'unknown'] || 0) + 1;
  const perWeek = weeks > 0 ? round2(total / weeks) : null;
  const deploy_frequency = { value: perWeek, per_week: perWeek, total, by_repo: byRepo, band: bandFor('deploy_frequency', perWeek) };
  if (weeks <= 0) guardrails.push({ code: 'empty_window', metric: 'deploy_frequency', reason: 'window has no length' });
  guard('deploy_frequency', total, 'deploys in window');

  // ---- lead time -----------------------------------------------------------
  const leads = [];
  for (const r of records) {
    if (!inWindow(r.deployed_at, w) || !Number.isFinite(r.first_commit_at) || r.first_commit_at > r.deployed_at) continue;
    leads.push(businessDays(r.first_commit_at, r.deployed_at));
  }
  const p50 = percentile(leads, 0.5);
  const lead_time = { value: p50, p50, p75: percentile(leads, 0.75), n: leads.length, band: bandFor('lead_time_days', p50) };
  guard('lead_time', leads.length, 'deployed records with a first commit');

  // ---- change failure rate + MTTR -----------------------------------------
  // Keys each tag shipped = records whose earliest deploy is that tag (by name,
  // else by identical timestamp).
  const shippedBy = new Map(); // deploy id -> Set(key)
  for (const r of records) {
    if (!Number.isFinite(r.deployed_at)) continue;
    const d = deployList.find((x) => (r.deployed_tag ? x.tags.includes(r.deployed_tag) : false) || x.ts === r.deployed_at)
      || deployList.find((x) => Math.floor(x.ts / DAY) === Math.floor(r.deployed_at / DAY));
    if (!d) continue;
    if (!shippedBy.has(d.id)) shippedBy.set(d.id, new Set());
    shippedBy.get(d.id).add(r.key);
  }
  const bugs = records
    .filter(isBug)
    .map((r) => ({ key: r.key, created: Number.isFinite(r.created) ? r.created : r.created_at, refs: refsOf(r) }))
    .filter((b) => Number.isFinite(b.created));
  const nextDeployAfter = (t, repo) => deployList.find((x) => x.ts > t && (repo == null || x.repo === repo));

  const evidence = [];
  const restores = [];
  const mttrEvidence = [];
  let failed = 0;
  for (const d of windowDeploys) {
    const signals = [];
    const hf = deployList.find((x) => x !== d && x.repo === d.repo && x.ts > d.ts && x.ts <= d.ts + failWinMs && x.kinds.has('hotfix'));
    if (hf) signals.push({ kind: 'hotfix_tag', at: hf.ts, ref: hf.tags.find(Boolean) });
    const shipped = shippedBy.get(d.id) || new Set();
    for (const b of bugs) {
      if (b.created <= d.ts || b.created > d.ts + failWinMs) continue;
      const hit = b.refs.find((k) => shipped.has(k));
      if (hit) signals.push({ kind: 'bug', at: b.created, ref: b.key, of: hit });
    }
    if (!signals.length) continue;
    failed++;
    signals.sort((a, b) => a.at - b.at);
    const first = signals[0];
    evidence.push({ deploy: d.tags[0], repo: d.repo, at: d.ts, signals });
    // Restore = next deploy strictly after the failure signal (a hotfix tag
    // signal is itself the restoring deploy, so look past the signal moment
    // only for bug signals).
    let restoredAt = null;
    if (first.kind === 'hotfix_tag') restoredAt = first.at;
    else {
      const nx = nextDeployAfter(first.at, d.repo);
      restoredAt = nx ? nx.ts : null;
    }
    const fromAt = first.kind === 'hotfix_tag' ? d.ts : first.at;
    if (restoredAt !== null) {
      const hours = round2((restoredAt - fromAt) / HOUR);
      restores.push(hours);
      mttrEvidence.push({ deploy: d.tags[0], repo: d.repo, from: fromAt, restored_at: restoredAt, hours, signal: first.ref });
    }
  }
  const cfr = total > 0 ? round2(failed / total) : null;
  const change_failure_rate = { value: cfr, failed, total, evidence, band: bandFor('change_failure_rate', cfr) };
  guard('change_failure_rate', total, 'deploys in window');

  const mttrP50 = percentile(restores, 0.5);
  const mttr = { value: mttrP50, p50: mttrP50, unit: 'hours', n: restores.length, evidence: mttrEvidence, band: bandFor('mttr_hours', mttrP50) };
  guard('mttr', restores.length, 'restored failures');

  return { deploy_frequency, lead_time, change_failure_rate, mttr, guardrails, bands: DORA_BANDS, window: w };
}

module.exports = { doraMetrics, DORA_BANDS, bandFor, percentile };
