// Flow metrics (LinearB / Swarmia-style) over the ticket corpus. Pure
// functions, zero I/O. Every metric returns the same envelope so the UI,
// reports and guardrails can treat them uniformly:
//   { value, n, coverage, weak_reasons: [] }
// `n` = items the value is computed from, `coverage` = n / eligible (0..1, or
// null when nothing is eligible), `weak_reasons` = machine codes explaining why
// the value should not be read at face value. Per-person rates always carry
// { capacity_days, time_off_days } — a ratio is never shown without its
// capacity context.
//
// Record fields used (all optional; absent → the item is skipped/uncovered):
//   key, type, priority, labels[], epic, assignee_id, subtask, scope_excluded,
//   created_at, started_at, done_at / eff_done_at (ms), dev_days,
//   estimate_days, actual_days, hotfix_linked, bucket,
//   phases: { design, dev, review, deployment, rework } in days (null = not tracked)
'use strict';

const C = require('./capacity.js');

const DAY = 86400000;
const MIN_N = 5;
const BUCKETS = [
  { id: '<0.5', lo: -Infinity, hi: 0.5 },
  { id: '0.5-0.8', lo: 0.5, hi: 0.8 },
  { id: '0.8-1.25', lo: 0.8, hi: 1.25 },
  { id: '1.25-2', lo: 1.25, hi: 2 },
  { id: '>2', lo: 2, hi: Infinity },
];
const PHASES = ['design', 'dev', 'review', 'deployment', 'rework'];

const doneAt = (r) => r.eff_done_at ?? r.done_at ?? null;
const inWin = (t, w) => t != null && t >= w.since && t < w.until;
const counted = (r) => !r.scope_excluded && !r.subtask;
const round = (x, d = 3) => (x == null ? null : Math.round(x * 10 ** d) / 10 ** d);

function quantile(xs, q) {
  const s = xs.filter(Number.isFinite).slice().sort((a, b) => a - b);
  if (!s.length) return null;
  const pos = (s.length - 1) * q;
  const lo = Math.floor(pos);
  const hi = Math.ceil(pos);
  return s[lo] + (s[hi] - s[lo]) * (pos - lo);
}
const median = (xs) => quantile(xs, 0.5);

/** Whole work days (Mon–Fri UTC) strictly after `from` up to `to`. */
function bizDaysBetween(from, to) {
  if (!(from < to)) return 0;
  let n = 0;
  for (let d = Math.floor(from / DAY) * DAY + DAY; d <= to; d += DAY) {
    const wd = new Date(d).getUTCDay();
    if (wd !== 0 && wd !== 6) n++;
  }
  return n;
}

function envelope(value, n, eligible, extra = {}, reasons = []) {
  const coverage = eligible > 0 ? n / eligible : null;
  const weak = [...reasons];
  if (n < MIN_N) weak.push('low_n');
  return { value, n, coverage: round(coverage), weak_reasons: [...new Set(weak)], ...extra };
}

/** Throughput per working week, counted and estimate-weighted. With `people`
 *  (id→person), also per-person rates per capacity day. */
function throughputPerWeek(records, window, opts = {}) {
  const weight = opts.weight || ((r) => Number(r.estimate_days) || 0);
  const eligible = records.filter((r) => !r.subtask && inWin(doneAt(r), window));
  const done = eligible.filter(counted);
  const business = C.workDaysIn(window, opts.capacity).size;
  const weeks = business / 5;
  const count = done.length;
  const weighted = done.reduce((a, r) => a + weight(r), 0);
  const unweighted = done.filter((r) => !(weight(r) > 0)).length;
  const per_person = {};
  if (opts.people) {
    const by = new Map();
    for (const r of done) {
      if (!r.assignee_id) continue;
      const e = by.get(r.assignee_id) || { count: 0, weighted: 0 };
      e.count++;
      e.weighted += weight(r);
      by.set(r.assignee_id, e);
    }
    for (const [id, p] of Object.entries(opts.people)) {
      const a = C.availability(p, window, opts.capacity);
      const e = by.get(id) || { count: 0, weighted: 0 };
      per_person[id] = {
        count: e.count,
        weighted: round(e.weighted),
        capacity_days: a.capacity_days,
        time_off_days: a.time_off_days,
        count_per_capacity_day: round(C.perCapacity(e.count, a.capacity_days)),
        weighted_per_capacity_day: round(C.perCapacity(e.weighted, a.capacity_days)),
      };
    }
  }
  const reasons = [];
  if (count && unweighted / count > 0.3) reasons.push('low_estimate_coverage');
  return envelope(
    { count, weighted: round(weighted), weeks: round(weeks), count_per_week: round(C.perCapacity(count, weeks)), weighted_per_week: round(C.perCapacity(weighted, weeks)) },
    count, eligible.length, { per_person }, reasons,
  );
}

/** Work in progress at `now`: started, not done, not excluded. */
function wip(records, opts = {}) {
  const now = opts.now ?? Date.now();
  const open = records.filter((r) => counted(r) && r.started_at != null && r.started_at <= now && !(doneAt(r) != null && doneAt(r) <= now));
  const by_person = {};
  for (const r of open) if (r.assignee_id) by_person[r.assignee_id] = (by_person[r.assignee_id] || 0) + 1;
  const people = Object.keys(by_person).length;
  return envelope({ count: open.length, by_person, per_person_mean: people ? round(open.length / people) : null }, open.length, open.length, {});
}

/** Open items older (business days since start) than the p75 cycle time of
 *  completed items in the same bucket (r.bucket || r.type). */
function agingWip(records, opts = {}) {
  const now = opts.now ?? Date.now();
  const bucketOf = (r) => r.bucket || r.type || 'other';
  const cycles = {};
  for (const r of records) {
    if (!counted(r) || r.started_at == null || doneAt(r) == null || doneAt(r) > now) continue;
    (cycles[bucketOf(r)] ||= []).push(bizDaysBetween(r.started_at, doneAt(r)));
  }
  const p75 = Object.fromEntries(Object.entries(cycles).map(([b, xs]) => [b, quantile(xs, 0.75)]));
  const open = records.filter((r) => counted(r) && r.started_at != null && r.started_at <= now && !(doneAt(r) != null && doneAt(r) <= now));
  const items = [];
  let judged = 0;
  for (const r of open) {
    const lim = p75[bucketOf(r)];
    if (lim == null) continue;
    judged++;
    const age = bizDaysBetween(r.started_at, now);
    if (age > lim) items.push({ key: r.key, bucket: bucketOf(r), assignee_id: r.assignee_id || null, age_days: age, p75_days: round(lim) });
  }
  items.sort((a, b) => b.age_days - a.age_days);
  const reasons = open.length > judged ? ['no_bucket_history'] : [];
  return envelope({ count: items.length, items, p75_by_bucket: p75 }, judged, open.length, {}, reasons);
}

/** Mean distinct ticket keys touched per person per active day.
 *  activity = [{ person, day: 'YYYY-MM-DD' | ms, key }] (commits, transitions…). */
function contextSwitching(activity) {
  const days = new Map(); // person|day -> Set(keys)
  for (const a of activity || []) {
    if (!a || !a.person || !a.key || a.day == null) continue;
    const d = typeof a.day === 'number' ? new Date(a.day).toISOString().slice(0, 10) : String(a.day).slice(0, 10);
    const k = `${a.person}\u0000${d}`;
    if (!days.has(k)) days.set(k, new Set());
    days.get(k).add(a.key);
  }
  const per = {};
  let sum = 0;
  for (const [k, set] of days) {
    const p = k.split('\u0000')[0];
    const e = (per[p] ||= { active_days: 0, keys: 0 });
    e.active_days++;
    e.keys += set.size;
    sum += set.size;
  }
  const by_person = Object.fromEntries(Object.entries(per).map(([p, e]) => [p, { active_days: e.active_days, mean_keys_per_day: round(e.keys / e.active_days) }]));
  return envelope(days.size ? round(sum / days.size) : null, days.size, days.size, { by_person });
}

/** Dev days split by issue type, epic and label (shares 0..1). */
function investmentMix(records, window) {
  const pool = records.filter((r) => counted(r) && (!window || inWin(doneAt(r), window) || inWin(r.started_at, window)));
  const withDev = pool.filter((r) => Number(r.dev_days) > 0);
  const total = withDev.reduce((a, r) => a + Number(r.dev_days), 0);
  const tally = (keyFn) => {
    const m = {};
    for (const r of withDev) for (const k of keyFn(r)) m[k] = (m[k] || 0) + Number(r.dev_days);
    return Object.fromEntries(Object.entries(m).sort((a, b) => b[1] - a[1]).map(([k, v]) => [k, { days: round(v), share: round(total ? v / total : 0) }]));
  };
  const value = {
    total_dev_days: round(total),
    by_type: tally((r) => [r.type || 'unknown']),
    by_epic: tally((r) => [r.epic || 'no epic']),
    // a ticket with several labels counts toward each (label shares can sum > 1)
    by_label: tally((r) => (r.labels && r.labels.length ? r.labels : ['no label'])),
  };
  const reasons = pool.length && withDev.length / pool.length < 0.7 ? ['no_git_evidence'] : [];
  return envelope(value, withDev.length, pool.length, {}, reasons);
}

/** Why an item is unplanned (null when planned). */
function unplannedReason(r, window) {
  if (String(r.type).toLowerCase() === 'bug') return 'bug';
  if (r.hotfix_linked) return 'hotfix';
  if (String(r.priority).toLowerCase() === 'highest') return 'highest_priority';
  if (r.created_at != null && r.started_at != null && r.created_at >= window.since && bizDaysBetween(r.created_at, r.started_at) <= 2) return 'interrupt';
  return null;
}

/** Share of items started or done in the window that were unplanned. */
function unplannedShare(records, window) {
  const pool = records.filter((r) => counted(r) && (inWin(r.started_at, window) || inWin(doneAt(r), window)));
  const by_reason = {};
  const keys = [];
  for (const r of pool) {
    const why = unplannedReason(r, window);
    if (!why) continue;
    by_reason[why] = (by_reason[why] || 0) + 1;
    keys.push(r.key);
  }
  return envelope(pool.length ? round(keys.length / pool.length) : null, pool.length, pool.length, { unplanned: keys.length, by_reason, keys });
}

/** actual/estimate ratio histogram over done items with both numbers. */
function estimateAccuracy(records, window) {
  const done = records.filter((r) => counted(r) && doneAt(r) != null && (!window || inWin(doneAt(r), window)));
  const ratios = [];
  for (const r of done) {
    const e = Number(r.estimate_days);
    const a = Number(r.actual_days);
    if (e > 0 && Number.isFinite(a) && a >= 0 && r.actual_days != null) ratios.push(a / e);
  }
  const histogram = Object.fromEntries(BUCKETS.map((b) => [b.id, 0]));
  for (const x of ratios) histogram[BUCKETS.find((b) => x >= b.lo && x < b.hi).id]++;
  const within = ratios.filter((x) => x >= 0.75 && x <= 1.25).length;
  const reasons = done.length && ratios.length / done.length < 0.7 ? ['low_estimate_coverage'] : [];
  return envelope(
    { histogram, pct_within_25: ratios.length ? round(within / ratios.length) : null, median_ratio: round(median(ratios)) },
    ratios.length, done.length, {}, reasons,
  );
}

/** Median days per phase over done items. A phase with no data is
 *  "not tracked" (null), never a fake 0. */
function cycleTimeByPhase(records, window) {
  const done = records.filter((r) => counted(r) && doneAt(r) != null && (!window || inWin(doneAt(r), window)));
  const withPh = done.filter((r) => r.phases && typeof r.phases === 'object');
  const value = {};
  const phase_n = {};
  for (const ph of PHASES) {
    const xs = withPh.map((r) => r.phases[ph]).filter((x) => x != null && Number.isFinite(Number(x))).map(Number);
    phase_n[ph] = xs.length;
    value[ph] = xs.length ? { median: round(median(xs)), p75: round(quantile(xs, 0.75)), n: xs.length } : { median: null, p75: null, n: 0, status: 'not_tracked' };
  }
  const totals = withPh.map((r) => PHASES.reduce((a, ph) => a + (Number(r.phases[ph]) || 0), 0));
  value.total = { median: round(median(totals)), n: totals.length };
  const reasons = [];
  if (withPh.length && phase_n.design / withPh.length < 0.5) reasons.push('design_not_tracked_share');
  if (withPh.length && phase_n.review / withPh.length < 0.5) reasons.push('no_pr_data');
  return envelope(value, withPh.length, done.length, { phase_n }, reasons);
}

module.exports = {
  MIN_N, BUCKETS, PHASES, quantile, median, bizDaysBetween, unplannedReason,
  throughputPerWeek, wip, agingWip, contextSwitching, investmentMix, unplannedShare, estimateAccuracy, cycleTimeByPhase,
};
