// Ticket phase model (pure). Splits one ticket's life into the phases the
// lead reads — DESIGN, DEV, REVIEW (pickup + in-review), QA (wait vs rework),
// DEPLOY and REWORK — in local, timezone-aware working days.
//
// Honesty rules:
//   design  — null + reason 'not_tracked' unless real evidence exists (a design
//             status, a design/spike/POC/research sub-task or linked ticket, or
//             assignee activity before the first In Progress). Never a fake 0.
//   dev     — In Progress-class status time until the PR opens (or Code
//             Review starts); commit days outside those windows fill the gaps
//             (statuses are often not moved). To-Do / backlog never count.
//   review  — PR open → first review (pickup) → merge; no PR → Code Review status.
//   qa      — counts as work (rework, added to dev) only when commits land on
//             ≥ qa_work_min_commit_days distinct local days inside QA;
//             otherwise it is wait.
//   deploy  — merge → first deploy tag (name contains deployed / hf / hotfix).
//   rework  — passthrough of the git-blame charge-back (in = others rewrote
//             this ticket's code, out = this ticket rewrote others').
'use strict';

const { businessDaysTz, distinctDays, dayKey, isWorkday, DEFAULT_WEEKEND } = require('./tz');

const RE_DESIGN_WORK = /design|spike|poc|research|investigat/i;
const RE_DESIGN_STATUS = /design|analys|refin|groom|spec|discover|shap|solution|architect/i;
const RE_REVIEW_STATUS = /review|\bpr\b|merge request/i;
const RE_QA_STATUS = /\bqa\b|quality|test|verif/i;
const RE_BACKLOG_STATUS = /to-?\s?do|next sprint|backlog|ready|open|candidate|new|triage/i;
const RE_DEV_STATUS = /progress|develop|implement|coding|doing|build|active/i;
const RE_DEPLOY_TAG = /deployed|hotfix|hf/i;

const round2 = (x) => Math.round(x * 100) / 100;
const num = (x) => (Number.isFinite(x) ? x : null);

/** Status class of a raw interval: design | dev | review | qa | backlog | other. */
function statusClass(iv) {
  const s = String(iv.status || '');
  if (iv.phase === 'design' || RE_DESIGN_STATUS.test(s)) return 'design';
  if (RE_REVIEW_STATUS.test(s)) return 'review';
  if (RE_QA_STATUS.test(s)) return 'qa';
  if (RE_BACKLOG_STATUS.test(s)) return 'backlog';
  if (RE_DEV_STATUS.test(s)) return 'dev';
  return iv.phase === 'implementation' ? 'dev' : 'other';
}

/** True when a deploy-like tag name: contains deployed / hf / hotfix (any case). */
function isDeployTag(name) {
  return RE_DEPLOY_TAG.test(String(name || ''));
}

/**
 * phasesFor(record, {prs, tags, cfg, tz, offDays})
 *   record.intervals  [{status, from, to, phase?}]  (raw Jira status intervals)
 *   record.commit_ts  [ms]                          (commits mentioning the key)
 *   record.subtasks / record.links  [{key, type, summary, days?, started_at?, done_at?}]
 *   record.activity   [{at, author_id}]              (comments / edits)
 *   record.rework     {in_days, out_days}            (from lib/rework.js)
 *   prs   [{opened_at, first_review_at?, merged_at?}] for this ticket
 *   tags  [{name, at}] deploy candidates (non-deploy names are ignored)
 *   cfg   {qa_work_min_commit_days=2, weekend=[6,0]}
 */
function phasesFor(record, { prs = [], tags = [], cfg = {}, tz = 'UTC', offDays } = {}) {
  const weekend = cfg.weekend || DEFAULT_WEEKEND;
  const bd = (a, b) => (num(a) !== null && num(b) !== null && b > a ? businessDaysTz(a, b, { tz, weekend, offDays }) : 0);
  const intervals = (record.intervals || []).filter((iv) => num(iv.from) !== null && num(iv.to) !== null && iv.to > iv.from)
    .map((iv) => ({ ...iv, cls: statusClass(iv) }))
    .sort((a, b) => a.from - b.from);
  const commits = (record.commit_ts || []).filter((t) => num(t) !== null).slice().sort((a, b) => a - b);

  // ---- PR timeline ---------------------------------------------------------
  const pr = prs.filter((p) => num(p.opened_at) !== null).sort((a, b) => a.opened_at - b.opened_at)[0] || null;
  const prOpen = pr ? pr.opened_at : null;
  const merged = prs.map((p) => num(p.merged_at)).filter((x) => x !== null).sort((a, b) => b - a)[0] ?? null;
  const firstReviewAt = intervals.find((iv) => iv.cls === 'review');
  const devEnd = prOpen ?? (firstReviewAt ? firstReviewAt.from : null);

  // ---- DESIGN --------------------------------------------------------------
  const firstDev = intervals.find((iv) => iv.cls === 'dev');
  const evidence = [];
  let designDays = 0;
  const designIvs = intervals.filter((iv) => iv.cls === 'design');
  if (designIvs.length) {
    evidence.push('status');
    for (const iv of designIvs) designDays += bd(iv.from, iv.to);
  }
  const relDays = (r) => (num(r.days) !== null ? r.days : bd(r.started_at, r.done_at));
  const isDesignItem = (r) => RE_DESIGN_WORK.test(String(r.type || '')) || RE_DESIGN_WORK.test(String(r.summary || ''));
  for (const [list, tag] of [[record.subtasks, 'subtask'], [record.links, 'linked']]) {
    const hits = (list || []).filter(isDesignItem);
    if (!hits.length) continue;
    evidence.push(tag);
    for (const r of hits) designDays += relDays(r);
  }
  if (!evidence.length && firstDev && record.assignee_id) {
    const pre = (record.activity || [])
      .filter((a) => a.author_id === record.assignee_id && num(a.at) !== null && a.at < firstDev.from)
      .sort((a, b) => a.at - b.at);
    if (pre.length) {
      evidence.push('pre_dev_activity');
      designDays += bd(pre[0].at, firstDev.from);
    }
  }
  const design = evidence.length
    ? { days: round2(designDays), evidence }
    : { days: null, evidence: [], reason: 'not_tracked' };

  // ---- QA (decided before dev: QA rework is dev time) ----------------------
  const qaMin = cfg.qa_work_min_commit_days ?? 2;
  let qaWait = 0;
  let qaRework = 0;
  const qaWindows = intervals.filter((iv) => iv.cls === 'qa');
  for (const iv of qaWindows) {
    const days = distinctDays(commits.filter((t) => t >= iv.from && t < iv.to), tz).length;
    if (days >= qaMin) qaRework += bd(iv.from, iv.to);
    else qaWait += bd(iv.from, iv.to);
  }

  // ---- DEV -----------------------------------------------------------------
  const sources = [];
  let devDays = 0;
  const covered = []; // [from, to) windows already counted as dev
  for (const iv of intervals) {
    if (iv.cls !== 'dev') continue;
    const to = devEnd !== null && iv.from < devEnd ? Math.min(iv.to, devEnd) : iv.to;
    if (devEnd !== null && iv.from >= devEnd) {
      // Back in progress after review (review feedback) — still dev work.
      devDays += bd(iv.from, iv.to);
      covered.push([iv.from, iv.to]);
      continue;
    }
    devDays += bd(iv.from, to);
    covered.push([iv.from, to]);
  }
  if (covered.length) sources.push('status');
  // Commit stretches: each local workday with a commit that no status window
  // (dev, QA-rework, review) already covers counts one day.
  const reviewEnd = merged ?? null;
  const skipWindows = [
    ...covered,
    ...qaWindows.map((iv) => [iv.from, iv.to]),
    ...(prOpen !== null && reviewEnd !== null ? [[prOpen, reviewEnd]] : []),
  ];
  const coveredDays = new Set();
  for (const [a, b] of covered) for (const t of commits) if (t >= a && t < b) coveredDays.add(dayKey(t, tz));
  const gapDays = new Set();
  for (const t of commits) {
    if (skipWindows.some(([a, b]) => t >= a && t < b)) continue;
    if (!isWorkday(t, tz, weekend)) continue;
    const k = dayKey(t, tz);
    if (offDays && offDays.has(k)) continue;
    if (!coveredDays.has(k)) gapDays.add(k);
  }
  if (gapDays.size) {
    devDays += gapDays.size;
    sources.push('commit_stretch');
  }
  devDays += qaRework;
  const dev = { days: round2(devDays), sources };

  // ---- REVIEW --------------------------------------------------------------
  let review;
  if (pr) {
    const firstRev = prs.map((p) => num(p.first_review_at)).filter((x) => x !== null && x >= prOpen).sort((a, b) => a - b)[0] ?? null;
    review = {
      pickup: firstRev !== null ? round2(bd(prOpen, firstRev)) : null,
      in_review: firstRev !== null && merged !== null ? round2(bd(firstRev, merged)) : null,
      total: merged !== null ? round2(bd(prOpen, merged)) : null,
      source: 'pr',
    };
  } else {
    const revIvs = intervals.filter((iv) => iv.cls === 'review');
    const total = revIvs.reduce((s, iv) => s + bd(iv.from, iv.to), 0);
    review = revIvs.length
      ? { pickup: null, in_review: round2(total), total: round2(total), source: 'status' }
      : { pickup: null, in_review: null, total: null, source: null };
  }

  // ---- DEPLOY --------------------------------------------------------------
  let deploy = { days: null };
  if (merged !== null) {
    const tag = tags.filter((t) => isDeployTag(t.name) && num(t.at) !== null && t.at >= merged).sort((a, b) => a.at - b.at)[0];
    if (tag) deploy = { days: round2(bd(merged, tag.at)), tag: tag.name };
  }

  // ---- REWORK --------------------------------------------------------------
  const rw = record.rework || {};
  const rework = { in_days: num(rw.in_days) ?? 0, out_days: num(rw.out_days) ?? 0 };

  return {
    design,
    dev,
    review,
    qa: { wait: round2(qaWait), rework: round2(qaRework), counted: qaRework > 0 },
    deploy,
    rework,
  };
}

function quantile(sorted, q) {
  if (!sorted.length) return null;
  const pos = (sorted.length - 1) * q;
  const lo = Math.floor(pos);
  const hi = Math.ceil(pos);
  return round2(sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo));
}

const SUMMARY_FIELDS = {
  design: (p) => p.design.days,
  dev: (p) => p.dev.days,
  review_pickup: (p) => p.review.pickup,
  review_in_review: (p) => p.review.in_review,
  review_total: (p) => p.review.total,
  qa_wait: (p) => p.qa.wait,
  qa_rework: (p) => p.qa.rework,
  deploy: (p) => p.deploy.days,
  rework_in: (p) => p.rework.in_days,
};

/**
 * Per-phase p50/p75 over NON-null values only, plus coverage % (share of
 * records where the phase is tracked). Accepts records carrying `.phases` or
 * bare phase objects.
 */
function phaseSummary(records) {
  const ps = (records || []).map((r) => (r && r.phases ? r.phases : r)).filter((p) => p && p.dev);
  const out = {};
  for (const [name, get] of Object.entries(SUMMARY_FIELDS)) {
    const vals = ps.map(get).filter((v) => num(v) !== null).sort((a, b) => a - b);
    out[name] = {
      n: vals.length,
      p50: quantile(vals, 0.5),
      p75: quantile(vals, 0.75),
      coverage: ps.length ? Math.round((vals.length / ps.length) * 100) : 0,
    };
  }
  return out;
}

module.exports = { phasesFor, phaseSummary, isDeployTag, statusClass };
