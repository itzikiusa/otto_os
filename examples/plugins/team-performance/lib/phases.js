// Ticket phase model (pure). Splits one ticket's life into the phases the
// lead reads — DESIGN, DEV, REVIEW (pickup + in-review), QA (wait vs rework),
// DEPLOY and REWORK — in local, timezone-aware working days.
//
// Honesty rules (a phase with no evidence is null + a reason, never a fake 0):
//   design  — 'not_tracked' unless real evidence exists: a design status, a
//             design/spike/POC/research sub-task or linked ticket, or assignee
//             activity on distinct workdays in the 10 workdays before the first
//             In Progress. Windows are UNIONED (never summed twice) and clipped
//             to before the first dev status. Evidence without any timing →
//             null + 'evidence_without_timing'.
//   dev     — In Progress-class status time minus each PR's own review window;
//             commit days outside status/QA/PR windows fill the gaps (statuses
//             are often not moved). To-Do / backlog never count; a Reopened
//             status is dev. QA rework (below) is added on top.
//   coding  — first commit → first PR opened (coding_days).
//   review  — per merged PR: pickup (open → first review), in-review (first
//             review → merge), open → merge. Ticket total = union of the PRs'
//             [open, merge) windows; pickup = median per-PR pickup. Declined /
//             still-open PRs are counted separately as 'unmerged'. No PR at all
//             → Code Review status time.
//   qa      — when commits land on ≥ qa_work_min_commit_days distinct local
//             days inside a QA window, those commit days are rework (dev) and
//             the remainder is wait; under the threshold it is all wait. No QA
//             window → null + 'no_qa_stage'.
//   deploy  — last merge (or done_git_at without a PR) → the earliest deploy
//             tag that REACHES the ticket's commits (gitscan deployed_at). Only
//             without that, a tag of the SAME repo dated after the merge, then a
//             "ready for deploy/release" status window. Else 'not_deployed'.
//   rework  — passthrough of the git-blame charge-back (in = others rewrote
//             this ticket's code, out = this ticket rewrote others'); null +
//             'no_git_blame' when blame never covered the ticket.
'use strict';

const { businessDaysTz, distinctDays, dayKey, isWorkday, DEFAULT_WEEKEND } = require('./tz');
const { isDeployTag } = require('./gitscan');

/** Design-like work (sub-task/linked type or title). Shared with lib/subtasks.js. */
const RE_DESIGN = /design|spike|\bpoc\b|research|investigat/i;
const RE_DESIGN_STATUS = /design|analys|refin|groom|spec|discover|shap|solution|architect/i;
const RE_REOPEN_STATUS = /re-?\s?open/i;
const RE_DEPLOY_WAIT_STATUS = /ready (for|to) (deploy|release|prod)|awaiting (deploy|release)|pending (deploy|release)|to (deploy|release)$|^(deploy|release) (queue|ready|pending)$/i;
const RE_REVIEW_STATUS = /review|\bpr\b|merge request/i;
const RE_QA_STATUS = /\bqa\b|quality|test|verif/i;
// Anchored: only a status that IS a backlog word (not "Re-opened", "Ready for QA").
const RE_BACKLOG_STATUS = /^(to[ -]?do|open|new|backlog|selected for development|next sprint|ready|ready for dev(elopment)?|candidate|triage)$/i;
const RE_DEV_STATUS = /progress|develop|implement|coding|doing|build|active/i;

const PRE_DEV_LOOKBACK_WORKDAYS = 10;

const round2 = (x) => Math.round(x * 100) / 100;
const num = (x) => (Number.isFinite(x) ? x : null);
const toMs = (v) => (v == null ? null : typeof v === 'number' ? num(v) : num(Date.parse(v)));

/** Status class of a raw interval: design | dev | review | qa | deploy_wait | backlog | other. */
function statusClass(iv) {
  const s = String(iv.status || '').trim();
  if (RE_REOPEN_STATUS.test(s)) return 'dev';
  if (RE_DEPLOY_WAIT_STATUS.test(s)) return 'deploy_wait';
  if (iv.phase === 'design' || RE_DESIGN_STATUS.test(s)) return 'design';
  if (RE_REVIEW_STATUS.test(s)) return 'review';
  if (RE_QA_STATUS.test(s)) return 'qa';
  if (RE_BACKLOG_STATUS.test(s)) return 'backlog';
  if (RE_DEV_STATUS.test(s)) return 'dev';
  return iv.phase === 'implementation' ? 'dev' : 'other';
}

/** Merge [from, to) windows into a sorted, non-overlapping list. */
function unionWindows(wins) {
  const ws = wins.filter(([a, b]) => num(a) !== null && num(b) !== null && b > a).sort((x, y) => x[0] - y[0]);
  const out = [];
  for (const [a, b] of ws) {
    const last = out[out.length - 1];
    if (last && a <= last[1]) last[1] = Math.max(last[1], b);
    else out.push([a, b]);
  }
  return out;
}

/** `wins` minus `cut` (both [from, to) lists). */
function subtractWindows(wins, cut) {
  const cuts = unionWindows(cut);
  const out = [];
  for (const [a0, b] of unionWindows(wins)) {
    let a = a0;
    for (const [c, d] of cuts) {
      if (d <= a || c >= b) continue;
      if (c > a) out.push([a, c]);
      a = Math.max(a, d);
      if (a >= b) break;
    }
    if (a < b) out.push([a, b]);
  }
  return out;
}

const inAny = (t, wins) => wins.some(([a, b]) => t >= a && t < b);

function median(vals) {
  const s = vals.filter((v) => num(v) !== null).sort((a, b) => a - b);
  if (!s.length) return null;
  const m = Math.floor(s.length / 2);
  return round2(s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2);
}

/**
 * phasesFor(record, {prs, tags, cfg, tz, offDays, lookup})
 *   record.intervals  [{status, from, to, phase?}]  (raw Jira status intervals)
 *   record.commit_ts  [ms]                          (commits mentioning the key)
 *   record.subtasks / record.links  [{key, type, summary, days?, started_at?, done_at?}]
 *   record.activity   [{at, author_id}]              (comments / edits)
 *   record.rework     {in_days, out_days} | null     (from lib/rework.js; null = no blame)
 *   record.deployed_at / deployed_tag / done_git_at  (gitscan reachability)
 *   record.repos      [repo name]                    (repos the key's commits live in)
 *   prs   [{opened_at, first_review_at?, merged_at?, state?, repo?, number?}]
 *   tags  [{name, at, repo, sha}] fallback deploy candidates
 *   cfg   {qa_work_min_commit_days=2, weekend=[6,0], deploy_tag_patterns?}
 *   lookup(key) → {started_at, done_at} | null       (timing of linked/sub-task items)
 */
function phasesFor(record, { prs = [], tags = [], cfg = {}, tz = 'UTC', offDays, lookup } = {}) {
  const weekend = cfg.weekend || DEFAULT_WEEKEND;
  const bd = (a, b) => (num(a) !== null && num(b) !== null && b > a ? businessDaysTz(a, b, { tz, weekend, offDays }) : 0);
  const bdWins = (wins) => wins.reduce((s, [a, b]) => s + bd(a, b), 0);
  const intervals = (record.intervals || []).map((iv) => ({ ...iv, from: toMs(iv.from), to: toMs(iv.to) }))
    .filter((iv) => iv.from !== null && iv.to !== null && iv.to > iv.from)
    .map((iv) => ({ ...iv, cls: statusClass(iv) }))
    .sort((a, b) => a.from - b.from);
  const commits = (record.commit_ts || []).map(toMs).filter((t) => t !== null).sort((a, b) => a - b);
  const winsOf = (cls) => intervals.filter((iv) => iv.cls === cls).map((iv) => [iv.from, iv.to]);

  // ---- PR timeline ---------------------------------------------------------
  const allPrs = (prs || []).map((p) => ({ ...p, opened_at: toMs(p.opened_at), first_review_at: toMs(p.first_review_at), merged_at: toMs(p.merged_at), updated_at: toMs(p.updated_at) }))
    .filter((p) => p.opened_at !== null)
    .sort((a, b) => a.opened_at - b.opened_at);
  const mergedPrs = allPrs.filter((p) => p.merged_at !== null && p.merged_at >= p.opened_at);
  const unmergedPrs = allPrs.filter((p) => !mergedPrs.includes(p));
  const firstPrOpen = allPrs.length ? allPrs[0].opened_at : null;
  const lastMerge = mergedPrs.length ? Math.max(...mergedPrs.map((p) => p.merged_at)) : null;
  // Each PR's own review window (unmerged: open → last update, best effort).
  const prWindows = allPrs.map((p) => [p.opened_at, p.merged_at ?? p.updated_at ?? p.opened_at]).filter(([a, b]) => b > a);

  // ---- DESIGN --------------------------------------------------------------
  const firstDevIv = intervals.find((iv) => iv.cls === 'dev');
  const firstDev = firstDevIv ? firstDevIv.from : null;
  const clip = (wins) => (firstDev === null ? wins : wins.map(([a, b]) => [a, Math.min(b, firstDev)]).filter(([a, b]) => b > a));
  const evidence = [];
  const designWins = [];
  let extraDesignDays = 0; // items with a day count but no window (cannot be unioned)
  let untimed = 0;
  const statusDesign = winsOf('design');
  if (statusDesign.length) {
    evidence.push('status');
    designWins.push(...statusDesign);
  }
  const isDesignItem = (r) => RE_DESIGN.test(String(r.type || '')) || RE_DESIGN.test(String(r.summary || ''));
  for (const [list, tag] of [[record.subtasks, 'subtask'], [record.links, 'linked']]) {
    const hits = (list || []).filter(isDesignItem);
    if (!hits.length) continue;
    evidence.push(tag);
    for (const r of hits) {
      const t = (typeof lookup === 'function' && r.key ? lookup(r.key) : null) || {};
      const a = toMs(r.started_at) ?? toMs(t.started_at);
      const b = toMs(r.done_at) ?? toMs(t.done_at);
      if (a !== null && b !== null && b > a) designWins.push([a, b]);
      else if (num(r.days) !== null) extraDesignDays += r.days;
      else untimed++;
    }
  }
  if (!evidence.length && firstDev !== null && record.assignee_id) {
    const backlog = winsOf('backlog');
    const days = new Set();
    for (const a of record.activity || []) {
      const at = toMs(a.at);
      if (a.author_id !== record.assignee_id || at === null || at >= firstDev) continue;
      if (inAny(at, backlog) || !isWorkday(at, tz, weekend)) continue;
      const k = dayKey(at, tz);
      if (offDays && offDays.has(k)) continue;
      if (bd(at, firstDev) > PRE_DEV_LOOKBACK_WORKDAYS) continue;
      days.add(k);
    }
    if (days.size) {
      evidence.push('pre_dev_activity');
      extraDesignDays += days.size;
    }
  }
  const timedWins = unionWindows(clip(designWins));
  const hasTiming = timedWins.length > 0 || extraDesignDays > 0;
  let design;
  if (!evidence.length) design = { days: null, evidence: [], reason: 'not_tracked' };
  else if (!hasTiming && untimed > 0) design = { days: null, evidence, reason: 'evidence_without_timing' };
  else design = { days: round2(bdWins(timedWins) + extraDesignDays), evidence };

  // ---- QA (decided before dev: QA rework is dev time) ----------------------
  const qaMin = cfg.qa_work_min_commit_days ?? 2;
  const qaWindows = winsOf('qa');
  let qa;
  let qaRework = 0;
  if (qaWindows.length) {
    let wait = 0;
    for (const [a, b] of qaWindows) {
      const span = bd(a, b);
      const days = distinctDays(commits.filter((t) => t >= a && t < b), tz).length;
      const rw = days >= qaMin ? Math.min(days, span) : 0;
      qaRework += rw;
      wait += span - rw;
    }
    qa = { wait: round2(wait), rework: round2(qaRework), counted: qaRework > 0 };
  } else {
    qa = { wait: null, rework: null, counted: false, reason: 'no_qa_stage' };
  }

  // ---- DEV -----------------------------------------------------------------
  const sources = [];
  const devWins = subtractWindows(winsOf('dev'), prWindows);
  let devDays = bdWins(devWins);
  if (devWins.length) sources.push('status');
  // Commit stretches: each local workday with a commit that no dev / QA / PR
  // window already covers counts one day (each PR skips only its own window).
  const skip = [...devWins, ...qaWindows, ...prWindows];
  const coveredDays = new Set(commits.filter((t) => inAny(t, devWins)).map((t) => dayKey(t, tz)));
  const gapDays = new Set();
  for (const t of commits) {
    if (inAny(t, skip) || !isWorkday(t, tz, weekend)) continue;
    const k = dayKey(t, tz);
    if (offDays && offDays.has(k)) continue;
    if (!coveredDays.has(k)) gapDays.add(k);
  }
  if (gapDays.size) {
    devDays += gapDays.size;
    sources.push('commit_stretch');
  }
  if (qaRework > 0) sources.push('qa_rework');
  devDays += qaRework;
  const dev = { days: round2(devDays), sources };
  const coding_days = commits.length && firstPrOpen !== null && commits[0] < firstPrOpen ? round2(bd(commits[0], firstPrOpen)) : null;

  // ---- REVIEW --------------------------------------------------------------
  let review;
  if (allPrs.length) {
    const perPr = mergedPrs.map((p) => {
      const fr = p.first_review_at !== null && p.first_review_at >= p.opened_at && p.first_review_at <= p.merged_at ? p.first_review_at : null;
      return {
        number: p.number ?? null,
        pickup: fr !== null ? round2(bd(p.opened_at, fr)) : null,
        in_review: fr !== null ? round2(bd(fr, p.merged_at)) : null,
        open_to_merge: round2(bd(p.opened_at, p.merged_at)),
        _fr: fr,
      };
    });
    const totalWins = unionWindows(mergedPrs.map((p) => [p.opened_at, p.merged_at]));
    const inRevWins = unionWindows(mergedPrs.map((p, i) => [perPr[i]._fr, p.merged_at]).filter(([a]) => a !== null));
    review = {
      pickup: median(perPr.map((p) => p.pickup)),
      in_review: inRevWins.length ? round2(bdWins(inRevWins)) : null,
      total: mergedPrs.length ? round2(bdWins(totalWins)) : null,
      source: 'pr',
      merged: mergedPrs.length,
      unmerged: unmergedPrs.length,
      prs: perPr.map(({ _fr, ...rest }) => rest),
    };
    if (!mergedPrs.length) review.state = 'unmerged';
  } else {
    const revWins = unionWindows(winsOf('review'));
    const total = round2(bdWins(revWins));
    review = revWins.length
      ? { pickup: null, in_review: total, total, source: 'status', merged: 0, unmerged: 0, prs: [] }
      : { pickup: null, in_review: null, total: null, source: null, merged: 0, unmerged: 0, prs: [] };
  }

  // ---- DEPLOY --------------------------------------------------------------
  const deployStart = lastMerge ?? toMs(record.done_git_at);
  const deployedAt = toMs(record.deployed_at);
  let deploy = null;
  if (deployedAt !== null) {
    deploy = deployStart !== null
      ? { days: round2(bd(deployStart, deployedAt)), tag: record.deployed_tag || null, source: 'reachability' }
      : { days: null, tag: record.deployed_tag || null, source: 'reachability', reason: 'no_merge' };
  } else if (deployStart !== null) {
    const repos = new Set([...allPrs.map((p) => p.repo), ...(record.repos || [])].filter(Boolean));
    const tag = (tags || [])
      .filter((t) => repos.has(t.repo) && isDeployTag(t.name, cfg.deploy_tag_patterns ? cfg : undefined) && num(t.at) !== null && t.at >= deployStart)
      .sort((a, b) => a.at - b.at)[0];
    if (tag) deploy = { days: round2(bd(deployStart, tag.at)), tag: tag.name, source: 'tag_same_repo' };
  }
  if (!deploy) {
    const waitWins = unionWindows(winsOf('deploy_wait'));
    deploy = waitWins.length
      ? { days: round2(bdWins(waitWins)), tag: null, source: 'status' }
      : { days: null, reason: 'not_deployed' };
  }

  // ---- REWORK --------------------------------------------------------------
  const rw = record.rework;
  const rework = rw && (num(rw.in_days) !== null || num(rw.out_days) !== null)
    ? { in_days: num(rw.in_days) ?? 0, out_days: num(rw.out_days) ?? 0 }
    : { in_days: null, out_days: null, reason: 'no_git_blame' };

  return { design, dev, coding_days, review, qa, deploy, rework };
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
  coding: (p) => p.coding_days,
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
 * Per-phase p50/p75 over NON-null values only. `n` = tickets where the phase
 * is tracked; `coverage` = n ÷ all tickets, ALWAYS a 0..1 fraction.
 * Accepts records carrying `.phases` or bare phase objects.
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
      coverage: ps.length ? Math.round((vals.length / ps.length) * 1000) / 1000 : 0,
    };
  }
  return out;
}

module.exports = { phasesFor, phaseSummary, statusClass, unionWindows, subtractWindows, RE_DESIGN, isDeployTag };
