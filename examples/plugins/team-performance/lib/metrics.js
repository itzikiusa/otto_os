// Integration glue: turns a loaded scope (corpus records + estimates + people)
// plus the git/PR side data into the metric suite the views and reports read
// (phases, DORA, flow, capacity, PR flow, rework, sub-tasks, guardrails).
// Pure apart from the inputs handed in — server.js does all the file I/O.
'use strict';

const A = require('./analytics.js');
const P = require('./phases.js');
const D = require('./dora.js');
const F = require('./flow.js');
const C = require('./capacity.js');
const G = require('./guardrails.js');
const R = require('./jira-rework.js');
const PR = require('./prs.js');

const DAY = 86400000;
const toMs = (v) => (v == null ? null : typeof v === 'number' ? v : Number.isFinite(Date.parse(v)) ? Date.parse(v) : null);
const round2 = (x) => (x == null || !Number.isFinite(x) ? null : Math.round(x * 100) / 100);

/** workweek ([1..5] weekday numbers) → weekend days for tz/capacity helpers. */
function weekendOf(workweek) {
  const ww = new Set(Array.isArray(workweek) && workweek.length ? workweek : [1, 2, 3, 4, 5]);
  return [0, 1, 2, 3, 4, 5, 6].filter((d) => !ww.has(d));
}

/** YYYY-MM-DD set of a person's time-off days (inclusive ranges). */
function offDaySet(person) {
  const out = new Set();
  for (const t of (person && person.time_off) || []) {
    const a = Date.parse(t.from);
    const b = Date.parse(t.to || t.from);
    if (!Number.isFinite(a) || !Number.isFinite(b)) continue;
    for (let d = a; d <= b && out.size < 5000; d += DAY) out.add(new Date(d).toISOString().slice(0, 10));
  }
  return out;
}

/** PRs (normalized, ISO times) grouped by ticket key, times converted to ms. */
function prsByKey(prs) {
  const m = new Map();
  for (const p of prs || []) {
    const row = {
      ...p,
      opened_at: toMs(p.opened_at),
      first_review_at: toMs(p.first_review_at),
      merged_at: toMs(p.merged_at),
    };
    for (const k of p.keys || []) {
      if (!m.has(k)) m.set(k, []);
      m.get(k).push(row);
    }
  }
  return m;
}

/** Attach `phases` (lib/phases shape) to every non-feature record. */
function attachPhases(records, { prMap, tags, config, people, canonical }) {
  const tz = config.timezone || 'UTC';
  const cfg = { weekend: weekendOf(config.workweek), qa_work_min_commit_days: config.qa_work_min_commit_days };
  const phaseTags = (tags || []).map((t) => ({ name: t.name, at: t.ts }));
  const offCache = new Map();
  const subsByParent = new Map();
  for (const r of records) if (r.subtask && r.parent_key) (subsByParent.get(r.parent_key) || subsByParent.set(r.parent_key, []).get(r.parent_key)).push(r);
  return records.map((r) => {
    if (r.feature) return r;
    const pid = r.assignee_id ? canonical(r.assignee_id) : null;
    if (pid && !offCache.has(pid)) offCache.set(pid, offDaySet(people[pid]));
    const subs = (subsByParent.get(r.key) || []).map((s) => ({ key: s.key, type: s.type, summary: s.summary, started_at: s.eff_start_at ?? s.first_active_at, done_at: s.eff_done_at ?? s.done_at }));
    const rec = { ...r, subtasks: subs, rework: { in_days: r.rework_in || 0, out_days: r.rework_out || 0 } };
    const phases = P.phasesFor(rec, { prs: (prMap && prMap.get(r.key)) || [], tags: phaseTags, cfg, tz, offDays: pid ? offCache.get(pid) : undefined });
    return { ...r, phases };
  });
}

/** Corpus record → the field names lib/flow.js reads. */
function flowRecord(r, estimates) {
  const e = estimates[r.key];
  const ph = r.phases;
  return {
    key: r.key,
    type: r.type,
    priority: r.priority || null,
    labels: r.labels || [],
    epic: r.epic_key || r.parent_key || null,
    assignee_id: r.assignee_id,
    subtask: Boolean(r.subtask),
    scope_excluded: Boolean(r.scope_excluded) || A.isExcluded(r),
    created_at: toMs(r.created),
    started_at: toMs(r.eff_start_at ?? r.first_active_at),
    done_at: A.isDone(r) ? toMs(r.eff_done_at ?? r.done_at) : null,
    dev_days: ph ? ph.dev.days : r.dev_days ?? null,
    estimate_days: e && e.days > 0 ? e.days : null,
    actual_days: A.isDone(r) ? A.actualDays(r) : null,
    hotfix_linked: r.deployed_kind === 'hotfix',
    bucket: r.type,
    phases: ph
      ? { design: ph.design.days, dev: ph.dev.days, review: ph.review.total, deployment: ph.deploy.days, rework: ph.rework.in_days || null }
      : null,
  };
}

/**
 * Full metric suite for a scope + window ({since, until} ms).
 * side = { tags, prs, target_ref_age_days, unmatched_authors, prs_status }.
 */
function computeMetrics(scope, window, side = {}) {
  const { config, estimates, people, canonical } = scope;
  const weekend = weekendOf(config.workweek);
  const recs = scope.records.filter((r) => !r.feature);
  const fr = recs.map((r) => flowRecord(r, estimates));
  const doneIn = fr.filter((r) => r.done_at != null && r.done_at >= window.since && r.done_at < window.until && !r.subtask);
  const delivered = doneIn.filter((r) => !r.scope_excluded);

  // Capacity (per canonical person who delivered or is registered+included).
  const ids = new Set();
  for (const r of recs) if (r.assignee_id) ids.add(canonical(r.assignee_id));
  const capPeople = {};
  for (const id of ids) {
    const fp = (scope.flat_people || {})[id];
    if (fp && fp.included === false) continue;
    capPeople[id] = { ...(people[id] || {}), id };
  }
  const capOpts = { weekend };
  const capacity = C.teamCapacity(capPeople, window, capOpts);
  for (const [id, a] of Object.entries(capacity.people)) a.name = ((scope.flat_people || {})[id] || {}).name || id;

  const weight = (r) => Number(r.estimate_days) || 0;
  const flow = {
    throughputPerWeek: F.throughputPerWeek(fr.map((r) => ({ ...r, assignee_id: r.assignee_id && canonical(r.assignee_id) })), window, { people: capPeople, weight, capacity: capOpts }),
    wip: F.wip(fr),
    agingWip: F.agingWip(fr),
    contextSwitching: F.contextSwitching(activityOf(recs, scope, window)),
    investmentMix: F.investmentMix(fr, window),
    unplannedShare: F.unplannedShare(fr, window),
    estimateAccuracy: F.estimateAccuracy(fr, window),
    cycleTimeByPhase: F.cycleTimeByPhase(fr, window),
  };

  const phaseRecs = recs.filter((r) => r.phases && A.isDone(r) && !r.subtask && (() => { const t = toMs(r.eff_done_at ?? r.done_at); return t >= window.since && t < window.until; })());
  const phases = P.phaseSummary(phaseRecs);

  const tags = side.tags || [];
  const dora = D.doraMetrics({
    records: recs.map((r) => ({ ...r, created: toMs(r.created), done_at: toMs(r.eff_done_at ?? r.done_at), deployed_at: toMs(r.deployed_at), first_commit_at: toMs(r.first_commit_at) })),
    tags,
    window: { start: window.since, end: window.until },
    cfg: { failure_window_days: 7, min_n: 5 },
  });

  const prsInWin = (side.prs || []).filter((p) => { const t = toMs(p.merged_at); return t != null && t >= window.since && t < window.until; });
  const pr_flow = { ...PR.prFlowSummary(prsInWin), total: prsInWin.length, approximated_times: prsInWin.filter((p) => p.opened_at_source === 'first_commit' || p.merged_at_source === 'updated_at').length };

  const reworkRecs = recs.filter((r) => { const t = toMs(r.eff_done_at ?? r.done_at); return A.isDone(r) && t >= window.since && t < window.until; });
  const rate = R.reworkRate(reworkRecs);
  const rework = {
    ...rate,
    items: recs.filter((r) => r.rework_of && reworkRecs.includes(r)).map((r) => ({ key: r.key, rework_of: r.rework_of, confidence: r.rework_confidence, signals: r.rework_signals || [], days: r.rework_out || 0 })).slice(0, 200),
  };

  const subtasks = recs.filter((r) => r.substantive_subtask).map((r) => ({
    key: r.key, parent_key: r.parent_key, summary: r.summary, assignee_id: r.assignee_id, assignee_name: r.assignee_name,
    credited_to: r.credited_to || r.assignee_id, rollup: r.rollup !== false, dev_days: r.phases ? r.phases.dev.days : r.dev_days ?? null,
  })).slice(0, 300);

  const staleRepo = Object.entries(side.target_ref_age_days || {}).filter(([, d]) => d > 14).sort((a, b) => b[1] - a[1])[0];
  const tagsInWin = tags.filter((t) => t.ts >= window.since && t.ts < window.until).length;
  const designTracked = phaseRecs.filter((r) => r.phases.design.days != null).length;
  const metricsForG = { ...flow, deploymentFrequency: { n: dora.deploy_frequency ? dora.deploy_frequency.total : tagsInWin } };
  const guardrails = G.evaluate(metricsForG, {
    delivered_count: delivered.length,
    pr_count: side.prs ? prsInWin.length : 0,
    design_tracked_share: phaseRecs.length ? designTracked / phaseRecs.length : null,
    estimate_coverage: delivered.length ? delivered.filter((r) => r.estimate_days > 0).length / delivered.length : null,
    git_evidence_share: delivered.length ? recs.filter((r) => delivered.some((d) => d.key === r.key) && r.git_change && r.git_change.commits).length / delivered.length : null,
    target_ref: staleRepo ? { name: staleRepo[0], stale: true, behind_days: Math.round(staleRepo[1]) } : null,
    unmapped_authors: (side.unmatched_authors || []).map((a) => a.name || a),
    capacity: { capacity_days: capacity.capacity_days, business_days: capacity.business_days },
    deploy_tags: tagsInWin,
  });
  for (const g of dora.guardrails || []) guardrails.push({ code: g.code, metric: `dora.${g.metric}`, severity: 'warning', reason: g.reason, action: 'Widen the period before comparing.' });
  if (pr_flow.approximated_times) {
    guardrails.push({ code: 'pr_times_approximated', metric: 'prPickup', severity: 'warning', reason: `${pr_flow.approximated_times} PR(s) have open/merge times approximated from commits or last update.`, action: 'Treat PR timings as indicative.' });
  }
  const badges = {};
  for (const k of [...Object.keys(flow), 'deploymentFrequency', 'leadTime', 'changeFailureRate', 'mttr', 'prPickup', 'prReview', 'prSize', 'rework']) {
    const b = G.badgeFor(k, guardrails);
    if (b) badges[k] = b;
  }

  return { window, dora, flow, capacity, phases, pr_flow, rework, subtasks, guardrails, badges, investment: flow.investmentMix.value, estimate_accuracy: flow.estimateAccuracy.value };
}

/** [{person, day, key}] from keyed commits + status transitions (canonical person). */
function activityOf(records, scope, window) {
  const out = [];
  for (const r of records) {
    if (!r.assignee_id || r.subtask) continue;
    const person = scope.canonical(r.assignee_id);
    for (const t of r.commit_ts || []) if (t >= window.since && t < window.until) out.push({ person, day: t, key: r.key });
    for (const iv of r.intervals || []) {
      const t = toMs(iv.from);
      if (t != null && t >= window.since && t < window.until) out.push({ person, day: t, key: r.key });
    }
  }
  return out;
}

/** Old-shape guardrails ([{id, level, msg}]) for the UI banner. */
function bannerOf(list) {
  return (list || []).map((g) => ({ id: `${g.code}:${g.metric}`, code: g.code, metric: g.metric, level: g.severity === 'danger' ? 'bad' : 'warn', msg: `${g.reason}${g.action ? ` ${g.action}` : ''}` }));
}

module.exports = { weekendOf, offDaySet, prsByKey, attachPhases, flowRecord, computeMetrics, activityOf, bannerOf, toMs, round2 };
