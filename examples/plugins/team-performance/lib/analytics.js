// Pure analytics core for the team-performance plugin: phase intervals,
// business-day math, baselines, verdicts, predictions, per-dev stats, goals.
// No I/O, no network — everything is deterministic on its inputs so the whole
// module is unit-testable (test/analytics.test.js).
'use strict';

const DAY = 86400000;
const WORKWEEK = [1, 2, 3, 4, 5]; // Mon–Fri (UTC day-of-week)

// ---------------------------------------------------------------------------
// Business-day math (all UTC; fractional days)
// ---------------------------------------------------------------------------

/** Fractional business days between two UTC ms timestamps. */
function businessDays(fromMs, toMs, workweek = WORKWEEK) {
  if (!(fromMs < toMs)) return 0;
  const wd = new Set(workweek);
  let total = 0;
  let dayStart = Math.floor(fromMs / DAY) * DAY;
  // Iterate UTC days overlapped by [from, to); sum only workday overlap.
  for (; dayStart < toMs; dayStart += DAY) {
    if (!wd.has(new Date(dayStart).getUTCDay())) continue;
    const s = Math.max(fromMs, dayStart);
    const e = Math.min(toMs, dayStart + DAY);
    if (e > s) total += (e - s) / DAY;
  }
  return total;
}

/** The UTC ms timestamp `days` business days after `fromMs` (skips non-workdays). */
function addBusinessDays(fromMs, days, workweek = WORKWEEK) {
  if (!(days > 0)) return fromMs;
  const wd = new Set(workweek);
  let cur = fromMs;
  let remaining = days;
  for (let guard = 0; guard < 40000 && remaining > 1e-9; guard++) {
    const dayStart = Math.floor(cur / DAY) * DAY;
    if (!wd.has(new Date(dayStart).getUTCDay())) {
      cur = dayStart + DAY; // jump to next midnight
      continue;
    }
    const available = (dayStart + DAY - cur) / DAY;
    const used = Math.min(available, remaining);
    cur += used * DAY;
    remaining -= used;
    if (remaining > 1e-9 && used === available) cur = dayStart + DAY;
  }
  return cur;
}

// ---------------------------------------------------------------------------
// Status → phase classification
// ---------------------------------------------------------------------------

const RE_DESIGN = /design|analys|refin|groom|spec|discover|shap|solution|architect/;
const RE_IMPL = /progress|develop|implement|coding|code review|review|test|qa|verif|merge|doing|build|active/;
const RE_WAIT = /to do|todo|open|backlog|blocked|waiting|hold|ready|triage|new/;
const RE_DONE = /done|closed|resolved|cancel|reject|complete|released|deploy/;

const PHASES = ['design', 'implementation', 'waiting', 'excluded'];

/**
 * Map a status name to a phase. Explicit `statusMap` overrides (matched
 * case-insensitively) win; then default regexes (design before implementation
 * so "Design Review" reads as design). Unknown statuses default to
 * `implementation` — counting unmapped active time as work is the safer error,
 * and the settings UI surfaces unmapped statuses for correction.
 */
function classifyStatus(name, statusMap = {}) {
  const lower = String(name || '').toLowerCase();
  for (const [k, v] of Object.entries(statusMap)) {
    if (k.toLowerCase() === lower && PHASES.includes(v)) return v;
  }
  if (RE_DONE.test(lower)) return 'excluded';
  if (RE_DESIGN.test(lower)) return 'design';
  if (RE_IMPL.test(lower)) return 'implementation';
  if (RE_WAIT.test(lower)) return 'waiting';
  return 'implementation';
}

// ---------------------------------------------------------------------------
// Changelog → status intervals
// ---------------------------------------------------------------------------

function ts(s) {
  const t = Date.parse(s);
  return Number.isNaN(t) ? null : t;
}

/** Chronological status transitions [{at, from, to}] from a Jira changelog. */
function statusTransitions(changelog) {
  const out = [];
  for (const h of (changelog && changelog.histories) || []) {
    const at = ts(h.created);
    if (at === null) continue;
    for (const it of h.items || []) {
      if (it.field === 'status') out.push({ at, from: it.fromString || 'Unknown', to: it.toString || 'Unknown' });
    }
  }
  out.sort((a, b) => a.at - b.at);
  return out;
}

/**
 * Build contiguous status intervals [{status, from, to}] for an issue:
 * created → t1 in the initial status (the first transition's `fromString`),
 * then one interval per transition; the last interval is open until `nowMs`.
 */
function buildIntervals(createdMs, changelog, nowMs) {
  const trans = statusTransitions(changelog);
  const intervals = [];
  let curStatus = trans.length ? trans[0].from : 'Unknown';
  let curFrom = createdMs;
  for (const t of trans) {
    const at = Math.max(t.at, curFrom); // guard clock skew
    if (at > curFrom) intervals.push({ status: curStatus, from: curFrom, to: at });
    curStatus = t.to;
    curFrom = at;
  }
  if (nowMs > curFrom) intervals.push({ status: curStatus, from: curFrom, to: nowMs });
  else intervals.push({ status: curStatus, from: curFrom, to: curFrom });
  return intervals;
}

/** Sum business days per phase over classified intervals. */
function phaseTotals(intervals, statusMap, workweek = WORKWEEK) {
  let design = 0;
  let impl = 0;
  let wait = 0;
  let firstActive = null;
  for (const iv of intervals) {
    const phase = classifyStatus(iv.status, statusMap);
    const d = businessDays(iv.from, iv.to, workweek);
    if (phase === 'design') design += d;
    else if (phase === 'implementation') impl += d;
    else if (phase === 'waiting') wait += d;
    if ((phase === 'design' || phase === 'implementation') && firstActive === null) firstActive = iv.from;
  }
  return { design_days: design, impl_days: impl, wait_days: wait, first_active_at: firstActive };
}

// ---------------------------------------------------------------------------
// Issue analysis
// ---------------------------------------------------------------------------

function isDoneName(name) {
  return RE_DONE.test(String(name || '').toLowerCase());
}

/** Last transition into a done-looking status (falls back for done_at). */
function lastDoneTransition(transitions, currentStatus, currentCategory) {
  let last = null;
  for (const t of transitions) {
    const doneish = isDoneName(t.to) || (currentCategory === 'done' && t.to === currentStatus);
    if (doneish) last = t.at;
    else if (last !== null) last = null; // reopened after done; keep only the final done run
  }
  return last;
}

/** The assignee accountId/name in effect at `atMs`, from changelog + current. */
function assigneeAt(changelog, current, atMs) {
  const changes = [];
  for (const h of (changelog && changelog.histories) || []) {
    const at = ts(h.created);
    if (at === null) continue;
    for (const it of h.items || []) {
      if (it.field === 'assignee') {
        changes.push({ at, fromId: it.from || null, fromName: it.fromString || null, toId: it.to || null, toName: it.toString || null });
      }
    }
  }
  if (!changes.length || atMs === null) return current;
  changes.sort((a, b) => a.at - b.at);
  let id = changes[0].fromId;
  let name = changes[0].fromName;
  for (const c of changes) {
    if (c.at > atMs) break;
    id = c.toId;
    name = c.toName;
  }
  return { accountId: id, displayName: name };
}

/**
 * Analyze one raw Jira issue (fields + changelog) into an IssueRecord.
 * opts: {statusMap, workweek, pointsField, gitIndex:{byKey,hasRepos},
 *        hasDesignStatuses, nowMs}
 */
function analyzeIssue(raw, opts) {
  const f = raw.fields || {};
  const createdMs = ts(f.created) ?? opts.nowMs;
  const currentStatus = f.status ? f.status.name : '';
  const currentCategory = f.status && f.status.statusCategory ? f.status.statusCategory.key : null;
  const intervals = buildIntervals(createdMs, raw.changelog, opts.nowMs);
  const transitions = statusTransitions(raw.changelog);

  // done_at: resolutiondate, else the last transition into a done-ish status.
  let doneAt = ts(f.resolutiondate);
  if (doneAt === null) doneAt = lastDoneTransition(transitions, currentStatus, currentCategory);
  // An issue whose current status is not done-category is open, whatever
  // stale resolution data says.
  if (currentCategory && currentCategory !== 'done' && ts(f.resolutiondate) === null) doneAt = null;

  // Phase totals only count time up to done (post-done drift is not work).
  const clipped = doneAt
    ? intervals
        .filter((iv) => iv.from < doneAt)
        .map((iv) => ({ ...iv, to: Math.min(iv.to, doneAt) }))
    : intervals;
  const totals = phaseTotals(clipped, opts.statusMap, opts.workweek);

  const cycle = doneAt !== null && totals.first_active_at !== null && totals.first_active_at < doneAt
    ? businessDays(totals.first_active_at, doneAt, opts.workweek)
    : null;
  const lead = doneAt !== null ? businessDays(createdMs, doneAt, opts.workweek) : null;

  const attributed = doneAt !== null ? assigneeAt(raw.changelog, f.assignee || null, doneAt) : f.assignee || null;

  const points = typeof f[opts.pointsField] === 'number' ? f[opts.pointsField] : null;
  const estimateDays = typeof f.timeoriginalestimate === 'number' && f.timeoriginalestimate > 0
    ? f.timeoriginalestimate / 28800 // Jira seconds → 8h workdays
    : null;

  const git = (opts.gitIndex && opts.gitIndex.byKey.get(raw.key)) || {};
  const deliveredAt = git.delivered_at ?? null;
  const firstCommitAt = git.first_commit_at ?? null;

  // Reopened: any transition out of a done-ish status.
  const reopened = transitions.some((t) => isDoneName(t.from) && !isDoneName(t.to));

  const flags = [];
  if (doneAt !== null && opts.gitIndex && opts.gitIndex.hasRepos && deliveredAt === null) flags.push('no_code');
  if (doneAt !== null && deliveredAt !== null && businessDays(doneAt, deliveredAt, opts.workweek) > 2) flags.push('late_merge');
  if (reopened) flags.push('reopened');
  if (points === null && estimateDays === null) flags.push('no_estimate');
  if (doneAt !== null && totals.design_days === 0 && opts.hasDesignStatuses) flags.push('skipped_design');

  const phased = intervals.map((iv) => ({ ...iv, phase: classifyStatus(iv.status, opts.statusMap) }));

  return {
    key: raw.key,
    type: f.issuetype ? f.issuetype.name : 'Unknown',
    summary: f.summary || '',
    status: currentStatus,
    status_category: currentCategory,
    assignee_id: attributed ? attributed.accountId : null,
    assignee_name: attributed ? attributed.displayName : null,
    created: createdMs,
    points,
    estimate_days: estimateDays,
    intervals: phased,
    design_days: round2(totals.design_days),
    impl_days: round2(totals.impl_days),
    wait_days: round2(totals.wait_days),
    first_active_at: totals.first_active_at,
    cycle_days: cycle !== null ? round2(cycle) : null,
    lead_days: lead !== null ? round2(lead) : null,
    first_commit_at: firstCommitAt,
    delivered_at: deliveredAt,
    done_at: doneAt,
    flags,
    updated: ts(f.updated),
  };
}

/**
 * Re-derive everything status-map-dependent on an existing record (after the
 * lead edits the map) from its stored raw-status intervals — no Jira refetch.
 */
function reanalyzeRecord(record, opts) {
  const doneAt = record.done_at;
  const raw = record.intervals || [];
  const clipped = doneAt
    ? raw.filter((iv) => iv.from < doneAt).map((iv) => ({ ...iv, to: Math.min(iv.to, doneAt) }))
    : raw;
  const totals = phaseTotals(clipped, opts.statusMap, opts.workweek);
  const cycle = doneAt !== null && totals.first_active_at !== null && totals.first_active_at < doneAt
    ? businessDays(totals.first_active_at, doneAt, opts.workweek)
    : null;
  const flags = record.flags.filter((fl) => fl !== 'skipped_design');
  if (doneAt !== null && totals.design_days === 0 && opts.hasDesignStatuses) flags.push('skipped_design');
  return {
    ...record,
    intervals: raw.map((iv) => ({ ...iv, phase: classifyStatus(iv.status, opts.statusMap) })),
    design_days: round2(totals.design_days),
    impl_days: round2(totals.impl_days),
    wait_days: round2(totals.wait_days),
    first_active_at: totals.first_active_at,
    cycle_days: cycle !== null ? round2(cycle) : null,
    flags,
  };
}

// ---------------------------------------------------------------------------
// Baselines ("how long it should have taken")
// ---------------------------------------------------------------------------

function percentile(sorted, q) {
  if (!sorted.length) return null;
  const idx = Math.max(0, Math.ceil((q / 100) * sorted.length) - 1); // nearest-rank
  return sorted[idx];
}

function stats3(values) {
  const s = values.slice().sort((a, b) => a - b);
  return { p25: percentile(s, 25), p50: percentile(s, 50), p75: percentile(s, 75) };
}

function bucketOf(recs) {
  return {
    n: recs.length,
    design: stats3(recs.map((r) => r.design_days ?? 0)),
    impl: stats3(recs.map((r) => r.impl_days ?? 0)),
    total: stats3(recs.map((r) => r.cycle_days)),
  };
}

const MIN_BUCKET = 3;

/**
 * Percentile buckets over completed records, keyed (type, points), with a
 * fallback chain (type+points → type → all) and a minimum sample size.
 */
function baselines(records) {
  const completed = records.filter((r) => r.done_at !== null && r.cycle_days !== null);
  const byTP = new Map();
  const byT = new Map();
  for (const r of completed) {
    const kp = `${r.type}|${r.points ?? 'unestimated'}`;
    if (!byTP.has(kp)) byTP.set(kp, []);
    byTP.get(kp).push(r);
    if (!byT.has(r.type)) byT.set(r.type, []);
    byT.get(r.type).push(r);
  }
  const buckets = [];
  for (const [k, recs] of byTP) {
    const [type, points] = k.split('|');
    buckets.push({ type, points: points === 'unestimated' ? null : Number(points), ...bucketOf(recs) });
  }
  buckets.sort((a, b) => a.type.localeCompare(b.type) || (a.points ?? 1e9) - (b.points ?? 1e9));

  function lookup(type, points) {
    const tp = byTP.get(`${type}|${points ?? 'unestimated'}`);
    if (tp && tp.length >= MIN_BUCKET) return { bucket: bucketOf(tp), level: 'type+points' };
    const t = byT.get(type);
    if (t && t.length >= MIN_BUCKET) return { bucket: bucketOf(t), level: 'type' };
    if (completed.length >= MIN_BUCKET) return { bucket: bucketOf(completed), level: 'all' };
    return null;
  }

  return { buckets, lookup, completed_n: completed.length };
}

/** fast / on_track / slow against a baseline p50 (lower is better). */
function verdict(actual, p50) {
  if (actual === null || actual === undefined || p50 === null || p50 === undefined) return null;
  if (p50 <= 0) return actual <= 0 ? 'on_track' : null;
  const ratio = actual / p50;
  if (ratio <= 0.8) return 'fast';
  if (ratio <= 1.25) return 'on_track';
  return 'slow';
}

// ---------------------------------------------------------------------------
// Predictions
// ---------------------------------------------------------------------------

function median(values) {
  if (!values.length) return null;
  const s = values.slice().sort((a, b) => a - b);
  const mid = Math.floor(s.length / 2);
  return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
}

/**
 * Per-assignee velocity factor: median of (actual cycle / bucket p50) over
 * their completed tasks; clamped [0.5, 2]; needs ≥3 samples else 1.0.
 */
function assigneeFactor(records, base) {
  const ratios = new Map();
  for (const r of records) {
    if (r.done_at === null || r.cycle_days === null || !r.assignee_id) continue;
    const hit = base.lookup(r.type, r.points);
    if (!hit || !hit.bucket.total.p50) continue;
    if (!ratios.has(r.assignee_id)) ratios.set(r.assignee_id, []);
    ratios.get(r.assignee_id).push(r.cycle_days / hit.bucket.total.p50);
  }
  const out = new Map();
  for (const [id, rs] of ratios) {
    if (rs.length >= 3) {
      out.set(id, { factor: Math.min(2, Math.max(0.5, median(rs))), n: rs.length });
    } else {
      out.set(id, { factor: 1.0, n: rs.length });
    }
  }
  return out;
}

function scale(st, factor) {
  return {
    p25: st.p25 !== null ? round2(st.p25 * factor) : null,
    p50: st.p50 !== null ? round2(st.p50 * factor) : null,
    p75: st.p75 !== null ? round2(st.p75 * factor) : null,
  };
}

/** Predicted timeline for a non-done record (null when no baseline exists). */
function predict(record, base, factorMap, nowMs, workweek = WORKWEEK) {
  const hit = base.lookup(record.type, record.points);
  if (!hit) return null;
  const f = record.assignee_id && factorMap.has(record.assignee_id) ? factorMap.get(record.assignee_id).factor : 1.0;
  const design = scale(hit.bucket.design, f);
  const impl = scale(hit.bucket.impl, f);
  const total = scale(hit.bucket.total, f);
  const elapsed = (record.design_days || 0) + (record.impl_days || 0);
  const pct = total.p50 ? round2(elapsed / total.p50) : null;
  const remaining = total.p50 !== null ? Math.max(0, total.p50 - elapsed) : null;
  return {
    design,
    impl,
    total,
    factor: f,
    based_on: hit.level,
    n: hit.bucket.n,
    elapsed_active_days: round2(elapsed),
    pct_consumed: pct,
    projected_done_at: remaining !== null ? addBusinessDays(nowMs, remaining, workweek) : null,
  };
}

// ---------------------------------------------------------------------------
// Per-assignee stats, goals
// ---------------------------------------------------------------------------

/** Average concurrent WIP over a dev's active windows (business days). */
function avgWip(windows, workweek, nowMs) {
  const spans = windows
    .map((w) => ({ from: w.from, to: w.to ?? nowMs }))
    .filter((w) => w.from !== null && w.to > w.from);
  if (!spans.length) return null;
  const lo = Math.min(...spans.map((w) => w.from));
  const hi = Math.max(...spans.map((w) => w.to));
  const span = businessDays(lo, hi, workweek);
  if (span <= 0) return null;
  const busy = spans.reduce((acc, w) => acc + businessDays(w.from, w.to, workweek), 0);
  return round2(busy / span);
}

/** Per-assignee summary stats over the corpus. */
function assigneeStats(records, base, workweek = WORKWEEK, nowMs = Date.now()) {
  const factorMap = assigneeFactor(records, base);
  const byDev = new Map();
  for (const r of records) {
    if (!r.assignee_id) continue;
    if (!byDev.has(r.assignee_id)) byDev.set(r.assignee_id, { name: r.assignee_name, completed: [], open: [] });
    const g = byDev.get(r.assignee_id);
    if (g.name === null || g.name === undefined) g.name = r.assignee_name;
    if (r.done_at !== null) g.completed.push(r);
    else g.open.push(r);
  }
  const out = [];
  for (const [id, g] of byDev) {
    const done = g.completed.filter((r) => r.cycle_days !== null).sort((a, b) => a.done_at - b.done_at);
    const cycles = done.map((r) => r.cycle_days);
    const mapes = done
      .filter((r) => r.estimate_days !== null && r.estimate_days > 0)
      .map((r) => Math.abs(r.cycle_days - r.estimate_days) / r.estimate_days);
    // Trend: median cycle of the last k vs the previous k (k = min(5, n/2)).
    let trend = null;
    if (cycles.length >= 4) {
      const k = Math.min(5, Math.floor(cycles.length / 2));
      const recent = median(cycles.slice(-k));
      const prev = median(cycles.slice(-2 * k, -k));
      if (prev > 0) {
        const ratio = recent / prev;
        trend = ratio < 0.9 ? 'improving' : ratio > 1.1 ? 'worsening' : 'flat';
      }
    }
    const windows = [
      ...done.map((r) => ({ from: r.first_active_at, to: r.done_at })),
      ...g.open.filter((r) => r.first_active_at).map((r) => ({ from: r.first_active_at, to: null })),
    ];
    const flags = {};
    for (const r of g.completed) for (const fl of r.flags || []) flags[fl] = (flags[fl] || 0) + 1;
    out.push({
      assignee_id: id,
      assignee_name: g.name || id,
      completed: g.completed.length,
      wip: g.open.filter((r) => r.first_active_at !== null).length,
      open: g.open.length,
      median_design: median(done.map((r) => r.design_days).filter((v) => v !== null)),
      median_impl: median(done.map((r) => r.impl_days).filter((v) => v !== null)),
      median_cycle: median(cycles),
      factor: factorMap.has(id) ? factorMap.get(id).factor : null,
      mape: mapes.length ? round2(median(mapes)) : null,
      avg_wip: avgWip(windows, workweek, nowMs),
      flags,
      trend,
    });
  }
  out.sort((a, b) => b.completed - a.completed || String(a.assignee_name).localeCompare(String(b.assignee_name)));
  return out;
}

const GOAL_METRICS = ['median_cycle_days', 'median_impl_days', 'median_design_days', 'estimate_mape', 'avg_wip'];

// stats-field name behind each goal metric.
const GOAL_SOURCE = {
  median_cycle_days: 'median_cycle',
  median_impl_days: 'median_impl',
  median_design_days: 'median_design',
  estimate_mape: 'mape',
  avg_wip: 'avg_wip',
};

/**
 * Suggested targets: step 10% toward the team median, never suggesting
 * regression for a dev already faster than the team —
 * target = min(own, max(team, own × 0.9)). All goal metrics are lower-better.
 */
function suggestGoals(devStats, teamMedians) {
  const out = [];
  for (const metric of GOAL_METRICS) {
    const own = devStats[GOAL_SOURCE[metric]] ?? devStats[metric] ?? null;
    const team = teamMedians[metric] ?? null;
    if (own === null || team === null) continue;
    const target = round2(Math.min(own, Math.max(team, own * 0.9)));
    // A zero target is unactionable (and unsaveable — targets must be > 0).
    if (target <= 0) continue;
    out.push({ metric, target });
  }
  return out;
}

/** Progress of one goal against current stats (lower-is-better). */
function goalProgress(goal, stats) {
  const current = stats[GOAL_SOURCE[goal.metric]] ?? stats[goal.metric] ?? null;
  return { current, met: current !== null && current <= goal.target };
}

/** Team-level medians used as the goal reference point. */
function teamMedians(stats) {
  const m = (k) => median(stats.map((s) => s[k]).filter((v) => v !== null && v !== undefined));
  return {
    median_cycle_days: m('median_cycle'),
    median_impl_days: m('median_impl'),
    median_design_days: m('median_design'),
    estimate_mape: m('mape'),
    avg_wip: m('avg_wip'),
  };
}

function round2(v) {
  return v === null || v === undefined ? v : Math.round(v * 100) / 100;
}

module.exports = {
  DAY,
  businessDays,
  addBusinessDays,
  classifyStatus,
  buildIntervals,
  phaseTotals,
  analyzeIssue,
  reanalyzeRecord,
  baselines,
  verdict,
  assigneeFactor,
  predict,
  assigneeStats,
  suggestGoals,
  goalProgress,
  teamMedians,
  median,
  GOAL_METRICS,
};
