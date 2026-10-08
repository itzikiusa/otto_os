// Integration glue: turns a loaded scope (corpus records + estimates + people)
// plus the git/PR side data into the metric suite the views and reports read
// (phases, DORA, flow, capacity, PR flow, rework, sub-tasks, guardrails).
// Pure apart from the inputs handed in — server.js does all the file I/O.
// Every `share` / `*_rate` / `coverage` in the output is a 0..1 FRACTION.
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

// TODO(scope-rules): lib/scope-rules.js (P1) owns weekendOf; drop the local
// copy once it has landed everywhere. Until then prefer it when present.
let SR = null;
try { SR = require('./scope-rules.js'); } catch { SR = null; }

/** workweek ([1..5] weekday numbers) → weekend days for tz/capacity helpers. */
function weekendOf(workweek) {
  if (SR && typeof SR.weekendOf === 'function') return SR.weekendOf(workweek);
  const ww = new Set(Array.isArray(workweek) && workweek.length ? workweek : [1, 2, 3, 4, 5]);
  return [0, 1, 2, 3, 4, 5, 6].filter((d) => !ww.has(d));
}

/** Strong rework = high confidence (explicit link / key in title + more), or a
 *  bug that rewrote the origin's code soon after delivery. Weak relates-to /
 *  keyword-only / self-reopened matches count only toward rate_all. */
const STRONG_REWORK_SIGNALS = new Set(['bug_after_delivery']);
const isStrongRework = (r) => r.rework_confidence === 'high' || (r.rework_signals || []).some((x) => STRONG_REWORK_SIGNALS.has(x));
const REWORK_ITEMS_CAP = 200;

/** DORA config from the plugin config + per-call side data (never hard-coded). */
function doraConfig(config = {}, side = {}, window, extra = {}) {
  const num = (v) => (v != null && Number(v) > 0 ? Number(v) : null);
  const cfg = {
    timezone: side.timezone || config.timezone || 'UTC',
    weekend: weekendOf(config.workweek),
    failure_window_days: num(side.bug_window_days) ?? num(config.failure_window_days) ?? 7,
    min_n: num(config.dora_min_n) ?? num(config.min_n) ?? 3,
    window: { start: window.since, end: window.until },
    prs: side.prs || [],
    deploy_tag_patterns: side.deploy_tag_patterns || config.deploy_tag_patterns,
    hotfix_tag_patterns: side.hotfix_tag_patterns || config.hotfix_tag_patterns || ['hf', 'hotfix'],
    branch: side.branch || config.branch || null,
    repos: side.repos || config.repos || undefined,
    ...extra,
  };
  if (typeof side.tagKind === 'function') cfg.tagKind = side.tagKind;
  if (typeof side.tagContains === 'function') cfg.tagContains = side.tagContains;
  return cfg;
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

/** [{at, author_id}] from a raw Jira issue's changelog + comments (when present). */
function activityOfRaw(raw) {
  const out = [];
  if (!raw) return out;
  const cl = raw.changelog;
  const histories = Array.isArray(cl) ? cl : (cl && cl.histories) || [];
  for (const h of histories) {
    const at = toMs(h.created ?? h.at);
    const who = (h.author && (h.author.accountId || h.author.id)) || h.author_id || null;
    if (at != null && who) out.push({ at, author_id: who });
  }
  const cm = raw.comments || (raw.fields && raw.fields.comment && raw.fields.comment.comments) || [];
  for (const c of Array.isArray(cm) ? cm : []) {
    const at = toMs(c.created ?? c.at);
    const who = (c.author && (c.author.accountId || c.author.id)) || c.author_id || null;
    if (at != null && who) out.push({ at, author_id: who });
  }
  if (Array.isArray(raw.activity)) for (const a of raw.activity) { const at = toMs(a.at); if (at != null && a.author_id) out.push({ at, author_id: a.author_id }); }
  return out.sort((x, y) => x.at - y.at);
}

/** A lead correction exists: manual actual days or an explicit effective window. */
const isLeadCorrected = (r) => r.manual_days != null || r.eff_start_override != null || r.eff_done_override != null;

/** Phase sum comparable to an actual: design + dev + review + QA wait (deploy is after done). */
function phaseSum(ph) {
  return [ph.design.days, ph.dev.days, ph.review.total, ph.qa.wait].reduce((s, v) => s + (Number.isFinite(v) ? v : 0), 0);
}

/**
 * Attach `phases` (lib/phases shape) to every non-feature record.
 * Optional `corpusIndex` (Map | object, key → raw corpus issue) supplies
 * changelog/comment activity and linked-item timing; records in `records`
 * are always indexed too.
 */
function attachPhases(records, { prMap, tags, config, people, canonical, corpusIndex } = {}) {
  config = config || {};
  canonical = canonical || ((x) => x);
  people = people || {};
  const tz = config.timezone || 'UTC';
  const cfg = { weekend: weekendOf(config.workweek), qa_work_min_commit_days: config.qa_work_min_commit_days };
  if (Array.isArray(config.deploy_tag_patterns)) cfg.deploy_tag_patterns = config.deploy_tag_patterns;
  // Keep repo + sha: the fallback only matches a tag of the ticket's own repo.
  const phaseTags = (tags || []).map((t) => ({ name: t.name, at: toMs(t.ts ?? t.at), repo: t.repo || null, sha: t.sha || null }));
  const offCache = new Map();
  const subsByParent = new Map();
  for (const r of records) if (r.subtask && r.parent_key) (subsByParent.get(r.parent_key) || subsByParent.set(r.parent_key, []).get(r.parent_key)).push(r);
  const byKey = new Map(records.map((r) => [r.key, r]));
  const rawOf = (k) => (corpusIndex ? (corpusIndex instanceof Map ? corpusIndex.get(k) : corpusIndex[k]) : null) || null;
  const lookup = (k) => {
    const x = byKey.get(k) || rawOf(k);
    if (!x) return null;
    return { started_at: toMs(x.eff_start_at ?? x.first_active_at), done_at: toMs(x.eff_done_at ?? x.done_at) };
  };
  return records.map((r) => {
    if (r.feature) return r;
    const pid = r.assignee_id ? canonical(r.assignee_id) : null;
    if (pid && !offCache.has(pid)) offCache.set(pid, offDaySet(people[pid]));
    const allSubs = subsByParent.get(r.key) || [];
    const subs = allSubs.map((s) => ({ key: s.key, type: s.type, summary: s.summary, started_at: s.eff_start_at ?? s.first_active_at, done_at: s.eff_done_at ?? s.done_at }));
    const raw = rawOf(r.key);
    const activity = r.activity || activityOfRaw(raw || r);
    const hasBlame = r.rework_in != null || r.rework_out != null;
    const rec = {
      ...r,
      subtasks: subs,
      links: r.links || (raw && raw.links) || [],
      activity,
      repos: ((r.git_change && r.git_change.repos) || []).map((x) => x.name).filter(Boolean),
      rework: hasBlame ? { in_days: r.rework_in || 0, out_days: r.rework_out || 0 } : null,
    };
    const corrected = isLeadCorrected(r);
    if (corrected) {
      // Lead override: phases live inside the corrected effective window.
      const a = toMs(r.eff_start_at);
      const b = toMs(r.eff_done_at);
      if (a != null && b != null && b > a) {
        rec.intervals = (r.intervals || []).map((iv) => ({ ...iv, from: Math.max(toMs(iv.from), a), to: Math.min(toMs(iv.to), b) })).filter((iv) => iv.to > iv.from);
        rec.commit_ts = (r.commit_ts || []).filter((t) => t >= a && t < b);
      }
    }
    const phases = P.phasesFor(rec, { prs: (prMap && prMap.get(r.key)) || [], tags: phaseTags, cfg, tz, offDays: pid ? offCache.get(pid) : undefined, lookup });
    phases.mismatch = false;
    if (corrected) {
      phases.dev.label = 'lead-corrected';
      const actual = r.manual_days ?? A.actualDays(r);
      const sum = phaseSum(phases);
      if (actual > 0 && Math.abs(sum - actual) / actual > 0.2) {
        phases.mismatch = true;
        phases.mismatch_detail = { phase_sum: round2(sum), corrected_actual: round2(actual) };
      }
    }
    const out = { ...r, phases };
    // Design sub-tasks are design, not dev: keep them out of the parent's child dev days.
    if (r.child_dev_days > 0) {
      const designDev = allSubs.filter((s) => P.RE_DESIGN.test(String(s.type || '')) || P.RE_DESIGN.test(String(s.summary || '')))
        .reduce((x, s) => x + (Number(s.dev_days) || 0), 0);
      if (designDev > 0) {
        out.child_dev_days = round2(Math.max(0, r.child_dev_days - designDev));
        out.child_dev_days_design_excluded = round2(designDev);
      }
    }
    return out;
  });
}

const PHASE_GUARD_FIELDS = ['design', 'coding', 'dev', 'review_pickup', 'review_total', 'qa_wait', 'qa_rework', 'deploy', 'rework_in'];

/** One guardrail per weak phase (n < 5 or coverage < 0.5), canonical id `phase_<name>_weak`. */
function phaseGuardrails(summary, { minN = 5, minCoverage = 0.5 } = {}) {
  const out = [];
  for (const name of PHASE_GUARD_FIELDS) {
    const s = summary && summary[name];
    if (!s) continue;
    if (s.n >= minN && s.coverage >= minCoverage) continue;
    const why = s.n < minN ? `only ${s.n} ticket(s) carry ${name.replace(/_/g, ' ')} timing` : `${name.replace(/_/g, ' ')} is tracked on only ${Math.round(s.coverage * 100)}% of tickets`;
    out.push({ code: `phase_${name}_weak`, metric: `phases.${name}`, severity: 'warning', reason: `Phase ${name.replace(/_/g, ' ')}: ${why}.`, action: 'Read this phase as indicative, not as a team figure.' });
  }
  return out;
}

/** Corpus record → the field names lib/flow.js reads. */
function flowRecord(r, estimates) {
  const e = estimates[r.key];
  const ph = r.phases;
  return {
    key: r.key,
    project: r.project || String(r.key).split('-')[0],
    assignee_name: r.assignee_name ?? null,
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
    summary: r.summary ?? null,
    deployed_at: toMs(r.deployed_at),
    rework_of: Array.isArray(r.rework_of) ? r.rework_of[0] || null : r.rework_of || null,
    sprints: r.sprints || [],
    sprint_changes: (r.sprint_changes || []).map((c) => ({ ...c, at: toMs(c.at) })),
    phases: ph
      ? ph
      : null,
  };
}

/**
 * Full metric suite for a scope + window ({since, until} ms).
 * side = { tags, prs, target_ref_age_days, unmatched_authors, prs_status,
 *          bug_window_days, timezone, deploy_tag_patterns, hotfix_tag_patterns,
 *          tagKind(name, cfg), tagContains(tag, sha), branch, repos, epics }.
 */
function computeMetrics(scope, window, side = {}) {
  const { config, estimates, people, canonical } = scope;
  const weekend = weekendOf(config.workweek);
  const recs = scope.records.filter((r) => !r.feature);
  const fr = recs.map((r) => flowRecord(r, estimates));
  const doneIn = fr.filter((r) => r.done_at != null && r.done_at >= window.since && r.done_at < window.until && !r.subtask);
  const delivered = doneIn.filter((r) => !r.scope_excluded);
  const deliveredKeys = new Set(delivered.map((r) => r.key));

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
  const capWeeks = capacity.capacity_days > 0 ? capacity.capacity_days / 5 : null;
  const activity = activityOf(recs, scope, window, side.prs);
  // ONE eligible population for the phase metrics: done in window, not a
  // sub-task, not scope-excluded, with phases attached.
  const phaseRecs = recs.filter((r) => r.phases && A.isDone(r) && !r.subtask && !(Boolean(r.scope_excluded) || A.isExcluded(r)) && (() => { const t = toMs(r.eff_done_at ?? r.done_at); return t >= window.since && t < window.until; })());
  const phaseKeys = new Set(phaseRecs.map((r) => r.key));
  const flow = {
    throughputPerWeek: F.throughputPerWeek(fr.map((r) => ({ ...r, assignee_id: r.assignee_id && canonical(r.assignee_id) })), window, { people: capPeople, weight, capacity: capOpts }),
    wip: F.wip(fr),
    agingWip: F.agingWip(fr),
    contextSwitching: F.contextSwitching(activity),
    investmentMix: F.investmentMix(fr, window),
    unplannedShare: F.unplannedShare(fr, window),
    estimateAccuracy: F.estimateAccuracy(fr, window),
    cycleTimeByPhase: { ...F.cycleTimeByPhase(fr.filter((r) => phaseKeys.has(r.key)), window), population: phaseKeys.size },
    flowEfficiency: F.flowEfficiency(fr, window),
    focusShare: F.focusShare(activity),
    escapeRate: F.escapeRate(fr, window),
    sprintPlanning: F.sprintPlanning(fr, (config.sprints || []).map((sp) => ({ ...sp, start: toMs(sp.start), end: toMs(sp.end) }))),
    reworkRateByPerson: F.reworkRateByPerson(fr.map((r) => ({ ...r, assignee_id: r.assignee_id && canonical(r.assignee_id) })), window),
    investmentByEpic: F.investmentByEpic(fr, window, side.epics || epicSummaries(scope.records)),
  };

  const phases = P.phaseSummary(phaseRecs);
  const phaseGuards = phaseGuardrails(phases);

  const tags = side.tags || [];
  const doraRecs = recs.map((r) => ({ ...r, created: toMs(r.created), done_at: toMs(r.eff_done_at ?? r.done_at), deployed_at: toMs(r.deployed_at), first_commit_at: toMs(r.first_commit_at), reopened_at: toMs(r.reopened_at) }));
  const dora = D.doraMetrics(doraRecs, tags, doraConfig(config, side, window, { capacity_days: capacity.capacity_days }));

  const prsInWin = (side.prs || []).filter((p) => { const t = toMs(p.merged_at); return t != null && t >= window.since && t < window.until; });
  const prCap = {};
  for (const [id, a] of Object.entries(capacity.people)) prCap[id] = { capacity_days: a.capacity_days };
  const prSummary = PR.prFlowSummary(prsInWin, { capacityDays: prCap });
  const pr_flow = {
    ...prSummary,
    total: prsInWin.length,
    approximated_times: prsInWin.filter((p) => p.opened_at_source === 'first_commit' || p.merged_at_source === 'updated_at').length,
    // Headline 0..1 share of merged PRs nobody reviewed; null without PR data.
    unreviewed_share: side.prs && prsInWin.length ? prSummary.unreviewed.share ?? null : null,
    merges_per_capacity_week: side.prs && capWeeks ? round2(prsInWin.length / capWeeks) : null,
  };
  const pr_people = PR.prByPerson(prsInWin, prCap);
  const review_load = PR.reviewLoad(pr_people);

  const reworkRecs = recs.filter((r) => { const t = toMs(r.eff_done_at ?? r.done_at); return A.isDone(r) && t >= window.since && t < window.until; });
  const reworkSet = new Set(reworkRecs);
  const rate = R.reworkRate(reworkRecs);
  const reworkAll = reworkRecs.filter((r) => !r.subtask && r.rework_of);
  const reworkStrong = reworkAll.filter(isStrongRework);
  const deliveredN = reworkRecs.filter((r) => !r.subtask).length;
  const bySignal = {};
  for (const r of reworkAll) for (const sg of new Set(r.rework_signals || [])) bySignal[sg] = (bySignal[sg] || 0) + 1;
  const reworkItems = recs.filter((r) => r.rework_of && reworkSet.has(r)).map((r) => ({ key: r.key, rework_of: r.rework_of, confidence: r.rework_confidence, signals: r.rework_signals || [], strong: isStrongRework(r), days: r.rework_out || 0 }));
  const rework = {
    ...rate,
    // Ticket-count shares (0..1) of done tickets that are rework of another:
    // rate_strong counts only high-confidence / strong-signal links.
    rate_all: deliveredN ? round2(reworkAll.length / deliveredN) : null,
    rate_strong: deliveredN ? round2(reworkStrong.length / deliveredN) : null,
    n: deliveredN,
    by_signal: bySignal,
    items: reworkItems.slice(0, REWORK_ITEMS_CAP),
    truncated: reworkItems.length > REWORK_ITEMS_CAP,
    total_items: reworkItems.length,
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
    git_evidence_share: delivered.length ? recs.filter((r) => deliveredKeys.has(r.key) && r.git_change && r.git_change.commits).length / delivered.length : null,
    target_ref: staleRepo ? { name: staleRepo[0], stale: true, behind_days: Math.round(staleRepo[1]) } : null,
    unmapped_authors: (side.unmatched_authors || []).map((a) => a.name || a),
    capacity: { capacity_days: capacity.capacity_days, business_days: capacity.business_days },
    deploy_tags: tagsInWin,
    pr_partial: side.prs ? prsInWin.filter((p) => p.partial).length : 0,
  });
  guardrails.push(...phaseGuards);
  for (const g of dora.guardrails || []) guardrails.push({ id: g.id, code: g.code, metric: `dora.${g.metric}`, severity: 'warning', reason: g.reason, action: 'Widen the period before comparing.' });
  if (typeof A.reworkGuardrails === 'function') for (const g of A.reworkGuardrails(recs) || []) guardrails.push({ severity: 'warning', ...g });
  const partialPrs = side.prs ? prsInWin.filter((p) => p.partial).length : 0;
  if (partialPrs && !guardrails.some((g) => g.code === 'pr_partial')) {
    guardrails.push({ code: 'pr_partial', metric: 'prSize', severity: 'warning', reason: `${partialPrs} PR(s) are missing commits or diff data (a daemon sub-call failed); they are refetched on the next scan.`, action: 'Treat PR size, review depth and rounds as incomplete.' });
  }
  if (pr_flow.approximated_times) {
    guardrails.push({ code: 'pr_times_approximated', metric: 'prPickup', severity: 'warning', reason: `${pr_flow.approximated_times} PR(s) have open/merge times approximated from commits or last update.`, action: 'Treat PR timings as indicative.' });
  }
  const badges = {};
  for (const k of [...Object.keys(flow), 'deploymentFrequency', 'leadTime', 'changeFailureRate', 'mttr', 'prPickup', 'prReview', 'prSize', 'rework']) {
    const b = G.badgeFor(k, guardrails);
    if (b) badges[k] = b;
  }

  return {
    window, dora, flow, capacity, phases, phase_population: phaseRecs.length, pr_flow, pr_people, review_load, rework, subtasks, guardrails, badges,
    investment: flow.investmentMix.value, estimate_accuracy: flow.estimateAccuracy.value,
  };
}

/** { epic_key: summary } from feature/epic records in the scope. */
function epicSummaries(records) {
  const out = {};
  for (const r of records || []) if (r && r.key && (r.feature || /epic/i.test(String(r.type || '')))) out[r.key] = r.summary ?? null;
  return out;
}

/**
 * [{person, day, key}] for focus / context-switching: keyed commits + status
 * transitions of the assignee, commits of SUBSTANTIVE sub-tasks (credited to
 * whoever did the work, keyed by the parent story) and PR review events
 * (each non-author reviewer/commenter, keyed by the PR's first ticket key).
 */
function activityOf(records, scope, window, prs) {
  const out = [];
  const inW = (t) => t != null && t >= window.since && t < window.until;
  for (const r of records) {
    if (r.subtask) {
      if (!r.substantive_subtask) continue;
      const who = r.credited_to || r.assignee_id;
      if (!who) continue;
      const person = scope.canonical(who);
      for (const t of r.commit_ts || []) if (inW(toMs(t))) out.push({ person, day: toMs(t), key: r.parent_key || r.key });
      continue;
    }
    if (!r.assignee_id) continue;
    const person = scope.canonical(r.assignee_id);
    for (const t of r.commit_ts || []) if (inW(toMs(t))) out.push({ person, day: toMs(t), key: r.key });
    for (const iv of r.intervals || []) {
      const t = toMs(iv.from);
      if (inW(t)) out.push({ person, day: t, key: r.key });
    }
  }
  for (const p of prs || []) {
    if (!p) continue;
    const key = (p.keys && p.keys[0]) || `PR#${p.number}`;
    for (const rv of p.reviewers || []) {
      const t = toMs(rv.reviewed_at);
      const who = rv.id ?? rv.name;
      if (!who || rv.name === p.author || !inW(t)) continue;
      out.push({ person: scope.canonical(who), day: t, key, source: 'pr_review' });
    }
  }
  return out;
}

/** Old-shape guardrails ([{id, level, msg}]) for the UI banner. */
function bannerOf(list) {
  return (list || []).map((g) => ({ id: /^phase_.+_weak$/.test(g.code || '') ? g.code : `${g.code}:${g.metric}`, code: g.code, metric: g.metric, level: g.severity === 'danger' ? 'bad' : 'warn', msg: `${g.reason}${g.action ? ` ${g.action}` : ''}` }));
}

module.exports = { doraConfig, epicSummaries, isStrongRework, weekendOf, offDaySet, prsByKey, attachPhases, activityOfRaw, phaseGuardrails, phaseSum, flowRecord, computeMetrics, activityOf, bannerOf, toMs, round2 };
