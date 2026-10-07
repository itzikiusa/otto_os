// Flow metrics (LinearB / Swarmia-style) over the ticket corpus. Pure
// functions, zero I/O. Every metric returns the same envelope so the UI,
// reports and guardrails can treat them uniformly:
//   { value, n, coverage, weak_reasons: [], guardrail }
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
//   deployed_at, rework_of, sprint_changes, summary,
//   phases: either the flat { design, dev, review, deployment, rework, … } in
//           days (null = not tracked) or phases.phasesFor() output — both are
//           read through phaseValues().
// `guardrail` = null when the metric is sound, else { level: 'weak', reasons }
// (same codes as weak_reasons) so the UI can banner it without re-deriving.
'use strict';

const C = require('./capacity.js');

const DAY = 86400000;
const MIN_N = 5;
const ACCURACY_MIN_N = 10;
const NO_EPIC = 'no_epic';
const TOP_UNLINKED = 10;
const BUCKETS = [
  { id: '<0.5', lo: -Infinity, hi: 0.5 },
  { id: '0.5-0.8', lo: 0.5, hi: 0.8 },
  { id: '0.8-1.25', lo: 0.8, hi: 1.25 },
  { id: '1.25-2', lo: 1.25, hi: 2 },
  { id: '>2', lo: 2, hi: Infinity },
];
// coding = dev minus QA-rework; dev = coding + qa_rework (phasesFor semantics).
// review = review_pickup + in_review when PR data exists.
const PHASES = ['design', 'coding', 'dev', 'review_pickup', 'in_review', 'review', 'qa_wait', 'qa_rework', 'deployment', 'rework'];

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
  const weak_reasons = [...new Set(weak)];
  const guardrail = weak_reasons.length ? { level: 'weak', reasons: weak_reasons } : null;
  return { value, n, coverage: round(coverage), weak_reasons, guardrail, ...extra };
}

const num = (x) => (x == null || x === '' || !Number.isFinite(Number(x)) ? null : Number(x));
const isObj = (x) => x != null && typeof x === 'object';

/** Normalise r.phases (flat numbers or phasesFor() objects) to
 *  { [PHASES[i]]: days | null }. null always means "not tracked". */
function phaseValues(r) {
  const p = r && r.phases;
  if (!isObj(p)) return null;
  if (isObj(p.dev)) {
    const dev = num(p.dev.days);
    const qaRw = isObj(p.qa) ? num(p.qa.rework) : null;
    const rv = isObj(p.review) ? p.review : {};
    return {
      design: isObj(p.design) ? num(p.design.days) : null,
      coding: dev != null ? Math.max(0, dev - (qaRw || 0)) : null,
      dev,
      review_pickup: num(rv.pickup),
      in_review: num(rv.in_review),
      review: num(rv.total),
      qa_wait: isObj(p.qa) ? num(p.qa.wait) : null,
      qa_rework: qaRw,
      deployment: isObj(p.deploy) ? num(p.deploy.days) : null,
      rework: isObj(p.rework) ? num(p.rework.in_days) : null,
    };
  }
  const dev = num(p.dev);
  const qaRw = num(p.qa_rework);
  return {
    design: num(p.design),
    coding: num(p.coding) ?? (dev != null ? Math.max(0, dev - (qaRw || 0)) : null),
    dev,
    review_pickup: num(p.review_pickup ?? p.pickup),
    in_review: num(p.in_review),
    review: num(p.review),
    qa_wait: num(p.qa_wait),
    qa_rework: qaRw,
    deployment: num(p.deployment ?? p.deploy),
    rework: num(p.rework),
  };
}

/** Elapsed business days start → deployed (or done). null when unknown. */
function elapsedDays(r) {
  const end = r.deployed_at ?? doneAt(r);
  if (r.started_at == null || end == null || end < r.started_at) return null;
  return bizDaysBetween(r.started_at, end);
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
  const tickets = [];
  for (const r of pool) {
    const why = unplannedReason(r, window);
    if (!why) continue;
    by_reason[why] = (by_reason[why] || 0) + 1;
    keys.push(r.key);
    tickets.push({ key: r.key, summary: r.summary ?? null, type: r.type ?? null, reason: why, assignee_id: r.assignee_id ?? null, dev_days: num(r.dev_days) });
  }
  tickets.sort((a, b) => (b.dev_days || 0) - (a.dev_days || 0) || String(a.key).localeCompare(String(b.key)));
  return envelope(pool.length ? round(keys.length / pool.length) : null, pool.length, pool.length, { unplanned: keys.length, by_reason, keys, tickets });
}

/** actual/estimate ratio distribution and actionable misses over the SAME
 * eligible done population. Preserve the histogram keys for report consumers;
 * bins/within_25/n are the overview UI's display contract. */
function estimateAccuracy(records, window) {
  const done = records.filter((r) => counted(r) && doneAt(r) != null && (!window || inWin(doneAt(r), window)));
  const pairs = [];
  for (const r of done) {
    const e = Number(r.estimate_days);
    const a = Number(r.actual_days);
    const ratio = a / e;
    if (Number.isFinite(e) && e > 0 && Number.isFinite(a) && a >= 0 && r.actual_days != null && Number.isFinite(ratio)) {
      pairs.push({ r, e, a, ratio });
    }
  }
  const ratios = pairs.map((p) => p.ratio);
  const histogram = Object.fromEntries(BUCKETS.map((b) => [b.id, 0]));
  for (const x of ratios) histogram[BUCKETS.find((b) => x >= b.lo && x < b.hi).id]++;
  const within = ratios.filter((x) => x >= 0.75 && x <= 1.25).length;
  const pctWithin = ratios.length ? round(within / ratios.length) : null;
  // Misses outside the inclusive ±25% band rank by proportional error in
  // either direction. Zero actual is observed data, with maximal log error;
  // only its finite ratio (0), never the internal Infinity, crosses the API.
  const worst = pairs.filter((p) => p.r.key && (p.ratio < 0.75 || p.ratio > 1.25))
    .sort((a, b) => Math.abs(Math.log(b.ratio)) - Math.abs(Math.log(a.ratio)) || String(a.r.key).localeCompare(String(b.r.key)))
    .slice(0, 30)
    .map(({ r, e, a, ratio }) => ({
      key: r.key, project: r.project || String(r.key).split('-')[0], summary: r.summary ?? null,
      assignee_id: r.assignee_id ?? null, assignee_name: r.assignee_name ?? null,
      est_days: e, actual_days: a, ratio: round(ratio),
    }));
  const reasons = done.length && ratios.length / done.length < 0.7 ? ['low_estimate_coverage'] : [];
  // A distribution of < 10 ratios is anecdote, not a shape.
  if (ratios.length < ACCURACY_MIN_N) reasons.push('low_sample');
  return envelope(
    {
      histogram, pct_within_25: pctWithin, median_ratio: round(median(ratios)),
      bins: BUCKETS.map((b) => ({ label: b.id, n: histogram[b.id] })),
      within_25: pctWithin, n: ratios.length, worst,
    },
    ratios.length, done.length, {}, reasons,
  );
}

/** Per-phase medians over done items, plus the per-ticket elapsed total.
 *  Each phase is reported on its own n/coverage; a phase with no data is
 *  "not tracked" (null), never a fake 0, and phases are never summed into the
 *  total — total = median of (start → deployed_at, else done) business days,
 *  because phases overlap and gaps between them are real waiting time. */
function cycleTimeByPhase(records, window) {
  const done = records.filter((r) => counted(r) && doneAt(r) != null && (!window || inWin(doneAt(r), window)));
  const withPh = done.map((r) => [r, phaseValues(r)]).filter(([, v]) => v);
  const value = {};
  const phase_n = {};
  for (const ph of PHASES) {
    const xs = withPh.map(([, v]) => v[ph]).filter((x) => x != null);
    phase_n[ph] = xs.length;
    const coverage = withPh.length ? round(xs.length / withPh.length) : null;
    value[ph] = xs.length
      ? { median: round(median(xs)), p75: round(quantile(xs, 0.75)), n: xs.length, coverage }
      : { median: null, p75: null, n: 0, coverage, status: 'not_tracked' };
  }
  const totals = done.map(elapsedDays).filter((x) => x != null);
  value.total = { median: round(median(totals)), p75: round(quantile(totals, 0.75)), n: totals.length, coverage: done.length ? round(totals.length / done.length) : null };
  const reasons = [];
  if (withPh.length && phase_n.design / withPh.length < 0.5) reasons.push('design_not_tracked_share');
  if (withPh.length && phase_n.review / withPh.length < 0.5) reasons.push('no_pr_data');
  if (done.length && totals.length / done.length < 0.7) reasons.push('no_start_time');
  return envelope(value, withPh.length, done.length, { phase_n }, reasons);
}

/** Flow efficiency = active (coding + in_review + qa_rework) / elapsed, summed
 *  over done items that have both (aggregate ratio, so long tickets weigh
 *  more — that is the point). Per-ticket active is capped at its elapsed. */
function flowEfficiency(records, window) {
  const done = records.filter((r) => counted(r) && doneAt(r) != null && (!window || inWin(doneAt(r), window)));
  let active = 0;
  let elapsed = 0;
  let n = 0;
  const per = [];
  for (const r of done) {
    const v = phaseValues(r);
    const el = elapsedDays(r);
    if (!v || v.coding == null || !(el > 0)) continue;
    const inRev = v.in_review ?? (v.review_pickup == null ? v.review : null) ?? 0;
    const a = Math.min(el, v.coding + inRev + (v.qa_rework || 0));
    active += a;
    elapsed += el;
    n++;
    per.push(a / el);
  }
  const reasons = [];
  if (n && per.filter((x) => x >= 1).length / n > 0.5) reasons.push('statuses_not_moved');
  return envelope(
    elapsed ? round(active / elapsed) : null, n, done.length,
    { active_days: round(active), elapsed_days: round(elapsed), median_ticket: round(median(per)) }, reasons,
  );
}

/** Focus share = person-days touching <= 1 ticket key / active person-days.
 *  activity = [{ person, day, key }] as for contextSwitching. */
function focusShare(activity) {
  const days = new Map();
  for (const a of activity || []) {
    if (!a || !a.person || !a.key || a.day == null) continue;
    const d = typeof a.day === 'number' ? new Date(a.day).toISOString().slice(0, 10) : String(a.day).slice(0, 10);
    const k = `${a.person}\u0000${d}`;
    if (!days.has(k)) days.set(k, new Set());
    days.get(k).add(a.key);
  }
  const per = {};
  let focused = 0;
  for (const [k, set] of days) {
    const p = k.split('\u0000')[0];
    const e = (per[p] ||= { active_days: 0, focused_days: 0 });
    e.active_days++;
    if (set.size <= 1) { e.focused_days++; focused++; }
  }
  const by_person = Object.fromEntries(Object.entries(per).map(([p, e]) => [p, { ...e, focus_share: round(e.focused_days / e.active_days) }]));
  return envelope(days.size ? round(focused / days.size) : null, days.size, days.size, { focused_days: focused, by_person });
}

const deliveredAt = (r) => r.deployed_at ?? doneAt(r);
const isBug = (r) => /bug|defect|incident/i.test(String(r.type || ''));

/** Escape rate = delivered-in-window items that a later bug points back at
 *  (rework_of) / items delivered in the window. */
function escapeRate(records, window) {
  const delivered = records.filter((r) => counted(r) && inWin(deliveredAt(r), window));
  const keys = new Set(delivered.map((r) => r.key));
  const escaped = new Map(); // origin key -> [bug keys]
  for (const r of records) {
    if (!isBug(r) || !r.rework_of || !keys.has(r.rework_of) || r.key === r.rework_of) continue;
    if (!escaped.has(r.rework_of)) escaped.set(r.rework_of, []);
    escaped.get(r.rework_of).push(r.key);
  }
  const items = [...escaped].map(([key, bugs]) => ({ key, bugs: bugs.sort() })).sort((a, b) => String(a.key).localeCompare(String(b.key)));
  const reasons = [];
  if (!records.some((r) => r.rework_of)) reasons.push('no_rework_links');
  return envelope(delivered.length ? round(escaped.size / delivered.length) : null, delivered.length, delivered.length, { escaped: escaped.size, items }, reasons);
}

/** Sprint names of a Jira "Sprint" changelog value: array or "A, B" string. */
function sprintList(v) {
  if (v == null) return [];
  const xs = Array.isArray(v) ? v : String(v).split(',');
  return xs.map((x) => String(x).trim()).filter(Boolean);
}

/** Was `r` in sprint `name` at time `t`? Replays r.sprint_changes
 *  ([{ at, from, to }]) — the state before the first change is its `from`,
 *  and with no history the current r.sprints[] holds throughout. */
function inSprintAt(r, name, t) {
  const ch = (r.sprint_changes || []).filter((c) => c && c.at != null).slice().sort((a, b) => a.at - b.at);
  if (!ch.length) return sprintList(r.sprints).includes(name) && (r.created_at == null || r.created_at <= t);
  let cur = sprintList(ch[0].from);
  if (r.created_at != null && r.created_at > t) return false;
  for (const c of ch) {
    if (c.at > t) break;
    cur = sprintList(c.to);
  }
  return cur.includes(name);
}
const everInSprint = (r, name) => sprintList(r.sprints).includes(name) || (r.sprint_changes || []).some((c) => sprintList(c.from).includes(name) || sprintList(c.to).includes(name));

/** Planning accuracy & scope creep from the Sprint field history.
 *  sprints = [{ name, start, end }] (ms). Per sprint:
 *    planned  = in the sprint at start; added = joined after start, before end;
 *    accuracy = planned items done by end / planned;
 *    creep    = added / planned. Top-level value = pooled over sprints. */
function sprintPlanning(records, sprints) {
  const rows = [];
  let planned = 0;
  let plannedDone = 0;
  let added = 0;
  for (const s of sprints || []) {
    if (!s || !s.name || s.start == null || s.end == null) continue;
    const pool = records.filter((r) => !r.subtask && everInSprint(r, s.name));
    const p = [];
    const a = [];
    for (const r of pool) {
      if (inSprintAt(r, s.name, s.start)) p.push(r);
      else if ((r.sprint_changes || []).some((c) => c.at > s.start && c.at <= s.end && sprintList(c.to).includes(s.name) && !sprintList(c.from).includes(s.name))) a.push(r);
    }
    const doneIn = (r) => doneAt(r) != null && doneAt(r) <= s.end;
    const pd = p.filter(doneIn);
    planned += p.length;
    plannedDone += pd.length;
    added += a.length;
    rows.push({
      name: s.name, planned: p.length, planned_done: pd.length, added: a.length, added_done: a.filter(doneIn).length,
      accuracy: p.length ? round(pd.length / p.length) : null,
      scope_creep: p.length ? round(a.length / p.length) : null,
      added_keys: a.map((r) => r.key).sort(),
      carried_over: p.filter((r) => !doneIn(r)).map((r) => r.key).sort(),
    });
  }
  const reasons = [];
  if (!records.some((r) => (r.sprint_changes || []).length)) reasons.push('no_sprint_history');
  return envelope(
    { accuracy: planned ? round(plannedDone / planned) : null, scope_creep: planned ? round(added / planned) : null, sprints: rows },
    planned, planned, { planned, planned_done: plannedDone, added }, reasons,
  );
}

/** Per-person rework rate over items delivered in the window:
 *  reworked_share = items a later ticket points back at (rework_of) / delivered;
 *  rework_days_ratio = rework charged back (phases.rework) / dev days. */
function reworkRateByPerson(records, window) {
  const delivered = records.filter((r) => counted(r) && r.assignee_id && inWin(deliveredAt(r), window));
  const origins = new Set(records.map((r) => r.rework_of).filter(Boolean));
  const per = {};
  for (const r of delivered) {
    const e = (per[r.assignee_id] ||= { delivered: 0, reworked: 0, rework_days: 0, dev_days: 0, keys: [] });
    e.delivered++;
    if (origins.has(r.key)) { e.reworked++; e.keys.push(r.key); }
    const v = phaseValues(r);
    if (v && v.rework != null) e.rework_days += v.rework;
    e.dev_days += Number(r.dev_days) || 0;
  }
  const by_person = {};
  let tot = 0;
  let totRw = 0;
  for (const [id, e] of Object.entries(per)) {
    tot += e.delivered;
    totRw += e.reworked;
    by_person[id] = {
      delivered: e.delivered, reworked: e.reworked, keys: e.keys.sort(),
      reworked_share: round(e.reworked / e.delivered),
      rework_days: round(e.rework_days), dev_days: round(e.dev_days),
      rework_days_ratio: e.dev_days > 0 ? round(e.rework_days / e.dev_days) : null,
      ...(e.delivered < MIN_N ? { guardrail: { level: 'weak', reasons: ['low_n'] } } : {}),
    };
  }
  return envelope(tot ? round(totRw / tot) : null, tot, delivered.length, { by_person });
}

/** Dev days by epic as rows, plus the biggest tickets with no epic.
 *  epics = { [epic_key]: summary } (from the corpus). */
function investmentByEpic(records, window, epics = {}) {
  const pool = records.filter((r) => counted(r) && Number(r.dev_days) > 0 && (!window || inWin(doneAt(r), window) || inWin(r.started_at, window)));
  const total = pool.reduce((a, r) => a + Number(r.dev_days), 0);
  const m = new Map();
  const unlinked = [];
  for (const r of pool) {
    const k = r.epic || NO_EPIC;
    m.set(k, (m.get(k) || 0) + Number(r.dev_days));
    if (!r.epic) unlinked.push({ key: r.key, summary: r.summary ?? null, dev_days: round(Number(r.dev_days)) });
  }
  const rows = [...m].map(([epic_key, d]) => ({
    epic_key,
    epic_summary: epic_key === NO_EPIC ? null : (epics[epic_key] ?? null),
    dev_days: round(d),
    share: round(total ? d / total : 0),
  })).sort((a, b) => b.dev_days - a.dev_days || String(a.epic_key).localeCompare(String(b.epic_key)));
  unlinked.sort((a, b) => b.dev_days - a.dev_days || String(a.key).localeCompare(String(b.key)));
  const noEpic = m.get(NO_EPIC) || 0;
  const reasons = total && noEpic / total > 0.3 ? ['high_unlinked_share'] : [];
  return envelope(
    { total_dev_days: round(total), rows, no_epic: { dev_days: round(noEpic), share: round(total ? noEpic / total : 0), top_tickets: unlinked.slice(0, TOP_UNLINKED) } },
    pool.length, pool.length, {}, reasons,
  );
}

module.exports = {
  MIN_N, ACCURACY_MIN_N, NO_EPIC, BUCKETS, PHASES, quantile, median, bizDaysBetween, unplannedReason, phaseValues, elapsedDays,
  throughputPerWeek, wip, agingWip, contextSwitching, investmentMix, unplannedShare, estimateAccuracy, cycleTimeByPhase,
  flowEfficiency, focusShare, escapeRate, sprintPlanning, reworkRateByPerson, investmentByEpic,
};
