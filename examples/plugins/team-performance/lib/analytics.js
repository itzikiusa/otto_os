// Pure analytics core for the team-performance plugin: phase intervals,
// business-day math, git-primary delivery timing, baselines, verdicts,
// predictions, per-dev stats (scope-weighted, multi-dev aware), routine
// detection, goals (per-dev, per-role, team scope).
// No I/O, no network — everything is deterministic on its inputs so the whole
// module is unit-testable (test/analytics.test.js).
//
// Timing model (git is the primary signal; Jira the secondary indication):
//   design          — Jira changelog (there are no commits during design)
//   implementation  — first commit → merge to develop/release (done_git_at);
//                     falls back to Jira status time when the task has no code
//   fixes           — key commits landing after done_git_at (incl. anything
//                     flowing to release/* or hotfix/* branches)
//   deploy wait     — end of fixing → *-DEPLOYED* tag (case-insensitive)
// A completed task with no git signal whose Jira-derived cycle exceeds
// `staleDays` is flagged `stale_timing` and excluded from medians/baselines
// (stale statuses must not poison the numbers).
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
// Git-derived timing (primary) + effective fields
// ---------------------------------------------------------------------------

const DEFAULT_STALE_DAYS = 45;

/** True when the record counts as completed (Jira done OR merged to target). */
function isDone(r) {
  return r.done_at !== null && r.done_at !== undefined
    ? true
    : r.done_git_at !== null && r.done_git_at !== undefined && r.done_git_at !== 0
      ? true
      : false;
}

/** Effective completion time: the merge (git) wins over the Jira transition. */
function effDoneAt(r) {
  return r.done_git_at ?? r.done_at ?? null;
}

/**
 * Apply one git-index entry to a record: raw git fields, derived durations,
 * effective (git-primary) timing, and all git-informed flags. Pure — returns a
 * new record. `gitEntry` may be undefined (no commits mention the key).
 * opts: {workweek, hasRepos, staleDays}
 */
function deriveGit(record, gitEntry, opts) {
  const g = gitEntry || {};
  const workweek = opts.workweek || WORKWEEK;
  const staleDays = opts.staleDays ?? DEFAULT_STALE_DAYS;

  const firstCommit = g.first_commit_at ?? null;
  const doneGit = g.done_git_at ?? null;
  const deployed = g.deployed_at ?? null;
  const lastFix = g.last_fix_at ?? null;

  const implGit = firstCommit !== null && doneGit !== null && firstCommit < doneGit
    ? round2(businessDays(firstCommit, doneGit, workweek))
    : firstCommit !== null && doneGit !== null
      ? 0
      : null;
  const fixDays = doneGit !== null && lastFix !== null && lastFix > doneGit
    ? round2(businessDays(doneGit, lastFix, workweek))
    : null;
  // Deployment step starts where fixing ends (no double counting of fix time).
  const deployFrom = doneGit !== null ? Math.max(doneGit, lastFix ?? doneGit) : null;
  const deployWait = deployFrom !== null && deployed !== null && deployed > deployFrom
    ? round2(businessDays(deployFrom, deployed, workweek))
    : null;

  const effDone = doneGit ?? record.done_at ?? null;
  const effStart = firstCommit !== null && record.first_active_at !== null && record.first_active_at !== undefined
    ? Math.min(firstCommit, record.first_active_at)
    : firstCommit ?? record.first_active_at ?? null;
  const effImpl = implGit ?? record.impl_days ?? null;
  const effCycle = effDone !== null && effStart !== null && effStart < effDone
    ? round2(businessDays(effStart, effDone, workweek))
    : effDone !== null && effStart !== null
      ? 0
      : null;

  const done = record.done_at !== null || doneGit !== null;
  const flags = (record.flags || []).filter(
    (f) => !['no_code', 'late_merge', 'unmerged_code', 'done_by_git_only', 'stale_timing', 'zero_time', 'multi_dev'].includes(f),
  );
  if (record.done_at !== null && opts.hasRepos && doneGit === null && firstCommit === null) flags.push('no_code');
  if (record.done_at !== null && opts.hasRepos && doneGit === null && firstCommit !== null) flags.push('unmerged_code');
  if (record.done_at !== null && doneGit !== null && businessDays(record.done_at, doneGit, workweek) > 2) flags.push('late_merge');
  if (doneGit !== null && record.done_at === null) flags.push('done_by_git_only');
  if (done && implGit === null && (record.cycle_days ?? 0) > staleDays) flags.push('stale_timing');
  // Bulk-closed junk: "done" with ~zero measured time and no git signal —
  // the definition of an outlier; excluded until a manual time is set.
  if (done && implGit === null && effCycle !== null && effCycle < 0.1) flags.push('zero_time');
  // Resurrected ancients: a stray recent commit/close on a years-old key makes
  // the effective cycle span years — one such record can poison the TEAM's
  // pooled pace. Anything over a working year is timing garbage, git or not.
  if (done && effCycle !== null && effCycle > 250) {
    if (!flags.includes('stale_timing')) flags.push('stale_timing');
  }

  const authors = Array.isArray(g.authors) ? g.authors : record.git_authors || [];
  // Multi-dev: ≥2 authors each carrying ≥25% of the key's commits (≥2 commits).
  const totalCommits = authors.reduce((a, x) => a + (x.commits || 0), 0);
  const heavy = authors.filter((x) => x.commits >= 2 && totalCommits > 0 && x.commits / totalCommits >= 0.25);
  if (heavy.length >= 2) flags.push('multi_dev');

  return {
    ...record,
    first_commit_at: firstCommit,
    delivered_at: g.delivered_at ?? record.delivered_at ?? null,
    done_git_at: doneGit,
    fix_count: g.fix_count ?? record.fix_count ?? 0,
    late_touches: g.late_touches ?? record.late_touches ?? 0,
    last_fix_at: lastFix,
    deployed_at: deployed,
    git_authors: authors,
    git_change: g.change ?? record.git_change ?? null,
    impl_days_git: implGit,
    fix_days: fixDays,
    deploy_wait_days: deployWait,
    eff_done_at: effDone,
    eff_start_at: effStart,
    eff_impl_days: effImpl !== null ? round2(effImpl) : null,
    eff_cycle_days: effCycle,
    timing_source: implGit !== null ? 'git' : 'jira',
    flags,
  };
}

// Effective accessors tolerating pre-v2 records (no eff_* fields yet).
// A per-story manual override (lead-entered actual days) beats everything.
const rImpl = (r) => r.eff_impl_days ?? r.impl_days ?? null;
// Actual time. A lead-entered manual time wins. Otherwise the base is the
// delivery cycle (first-active/commit → merge, NO fixes). Fix time is FOLDED
// IN only when the fixing was substantial real work — `include_fixes` is
// resolved upstream (explicit per-task override, else auto by fix-commit
// count) so a stray one-off touch never inflates a task but a genuine 10-commit
// fixing effort does.
const rCycle = (r) => {
  if (r.manual_days != null) return r.manual_days;
  const base = r.eff_cycle_days ?? r.cycle_days ?? null;
  if (base != null && r.include_fixes && r.fix_days) return round2(base + r.fix_days);
  return base;
};
const rDoneAt = (r) => r.eff_done_at ?? r.done_at ?? null;
const isStale = (r) => (r.flags || []).includes('stale_timing') || (r.flags || []).includes('zero_time');
// Excluded from every median/baseline/throughput: a lead-excluded story
// (excluded_override — hard, wins over everything), stale timing, the lead
// marked the story as an outlier, or a dev sub-task rolled up into its parent
// story (counting both would double the same work). A manual time override
// cures stale/outlier — the story re-enters at the entered value.
const isExcluded = (r) =>
  r.excluded_override === true || r.rollup === true || (r.manual_days == null && (r.outlier === true || isStale(r)));
// Timing-sample guard: "done" records with ~zero measured time are bulk-closed
// Jira junk, not measurements — they poison medians and pace ratios toward 0.
// A manual override is always a deliberate sample.
const isTimingSample = (r) => rCycle(r) !== null && (r.manual_days != null || rCycle(r) >= 0.1 || r.timing_source === 'git');
// Period filter: completed-sample window on the effective done time
// ([since, until); either bound optional).
const inPeriod = (r, sinceMs, untilMs) => {
  const d = rDoneAt(r) ?? 0;
  return (!sinceMs || d >= sinceMs) && (!untilMs || d < untilMs);
};

// ---------------------------------------------------------------------------
// Hierarchy: sub-task rollups + design-from-sub-tasks
// ---------------------------------------------------------------------------

const RE_DESIGN_TYPE = /design/i;

/**
 * Corpus-wide hierarchy pass (pure, view-time):
 * - Dev-typed sub-tasks whose parent story is in the corpus get `rollup: true`
 *   — the story (git-timed, AI-estimated) is the work item; counting its dev
 *   sub-tasks too would double the same work. QA/design/other sub-tasks stay
 *   standalone work items (that's their assignee's real work).
 * - Design-typed sub-tasks additionally paint the parent's design phase:
 *   parent `design_days_eff` = own design_days + Σ child design time.
 */
function enrichHierarchy(records) {
  const parents = new Map();
  for (const r of records) if (!r.subtask) parents.set(r.key, r);
  const designByParent = new Map();
  const out = records.map((r) => {
    if (!r.subtask || !r.parent_key || !parents.has(r.parent_key)) return r;
    // A sub-task that CONTAINS the word "design" (type or anywhere in the
    // summary) is design work for its parent story.
    const isDesign = RE_DESIGN_TYPE.test(r.type) || RE_DESIGN_TYPE.test(r.summary || '');
    if (isDesign) {
      const t = rCycle(r) ?? rImpl(r) ?? 0;
      designByParent.set(r.parent_key, (designByParent.get(r.parent_key) || 0) + t);
    }
    // EVERY sub-task (dev, QA, design, …) rolls up into its parent story —
    // the story is the unit of work; counting its breakdown too would double
    // it and a pile of tiny QA/dev sub-tasks would drown the real stories.
    return { ...r, rollup: true };
  });
  if (!designByParent.size) return out;
  return out.map((r) =>
    designByParent.has(r.key)
      ? { ...r, design_days_eff: round2((r.design_days || 0) + designByParent.get(r.key)) }
      : r,
  );
}

const rDesign = (r) => r.design_days_eff ?? r.design_days ?? null;

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
 *        hasDesignStatuses, nowMs, staleDays, descText?: (adf)=>string}
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
  const parentKey = f.parent && f.parent.key ? f.parent.key : null;
  const isSubtask = Boolean(f.issuetype && f.issuetype.subtask);

  // Reopened: any transition out of a done-ish status.
  const reopened = transitions.some((t) => isDoneName(t.from) && !isDoneName(t.to));

  const flags = [];
  if (reopened) flags.push('reopened');
  if (points === null && estimateDays === null) flags.push('no_estimate');
  if (doneAt !== null && totals.design_days === 0 && opts.hasDesignStatuses) flags.push('skipped_design');

  const phased = intervals.map((iv) => ({ ...iv, phase: classifyStatus(iv.status, opts.statusMap) }));

  const descText = opts.descText && f.description ? opts.descText(f.description) : '';

  const base = {
    key: raw.key,
    project: String(raw.key || '').split('-')[0] || null,
    parent_key: parentKey,
    subtask: isSubtask,
    type: f.issuetype ? f.issuetype.name : 'Unknown',
    summary: f.summary || '',
    description_snippet: descText ? descText.replace(/\s+/g, ' ').trim().slice(0, 1500) : (raw.description_snippet || ''),
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
    done_at: doneAt,
    flags,
    updated: ts(f.updated),
  };
  return deriveGit(base, opts.gitIndex ? opts.gitIndex.byKey.get(raw.key) : undefined, {
    workweek: opts.workweek,
    hasRepos: Boolean(opts.gitIndex && opts.gitIndex.hasRepos),
    staleDays: opts.staleDays,
  });
}

/**
 * Re-derive everything status-map-dependent on an existing record (after the
 * lead edits the map) from its stored raw-status intervals — no Jira refetch.
 * Git-derived fields are re-derived from the record's own stored git fields.
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
  const flags = (record.flags || []).filter((fl) => fl !== 'skipped_design');
  if (doneAt !== null && totals.design_days === 0 && opts.hasDesignStatuses) flags.push('skipped_design');
  const next = {
    ...record,
    intervals: raw.map((iv) => ({ ...iv, phase: classifyStatus(iv.status, opts.statusMap) })),
    design_days: round2(totals.design_days),
    impl_days: round2(totals.impl_days),
    wait_days: round2(totals.wait_days),
    first_active_at: totals.first_active_at,
    cycle_days: cycle !== null ? round2(cycle) : null,
    flags,
  };
  return deriveGit(
    next,
    {
      first_commit_at: next.first_commit_at ?? null,
      done_git_at: next.done_git_at ?? null,
      delivered_at: next.delivered_at ?? null,
      last_fix_at: next.last_fix_at ?? null,
      fix_count: next.fix_count ?? 0,
      deployed_at: next.deployed_at ?? null,
      authors: next.git_authors || [],
    },
    { workweek: opts.workweek, hasRepos: opts.hasRepos ?? true, staleDays: opts.staleDays },
  );
}

// ---------------------------------------------------------------------------
// Routine-work detection (version bumps & other repetitive tasks)
// ---------------------------------------------------------------------------

/** Normalize a summary into a repetition signature: digits/keys → '#'. */
function routineSignature(summary) {
  return String(summary || '')
    .toLowerCase()
    .replace(/\b[a-z][a-z0-9]+-\d+\b/g, '')
    .replace(/\d+(\.\d+)*/g, '#')
    .replace(/[^a-z#]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

const ROUTINE_GROUP_MIN = 4;

/** Map signature → count over the corpus (completed + open). */
function routineSignatures(records) {
  const counts = new Map();
  for (const r of records) {
    const sig = routineSignature(r.summary);
    if (!sig) continue;
    counts.set(sig, (counts.get(sig) || 0) + 1);
  }
  return counts;
}

/** A record is routine when the AI says so or its summary repeats ≥4 times. */
function isRoutine(record, sigCounts, est) {
  if (est && est.routine) return true;
  if (!sigCounts) return false;
  return (sigCounts.get(routineSignature(record.summary)) || 0) >= ROUTINE_GROUP_MIN;
}

// ---------------------------------------------------------------------------
// People registry: author matching + contributor credit (multi-dev)
// ---------------------------------------------------------------------------

function normName(s) {
  return String(s || '')
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')
    .replace(/[^a-z0-9@. ]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

/**
 * Canonical-id resolver for merged people: registry entries may carry
 * `merged_into: <otherId>` (one person, several Jira accounts over the years).
 * Chains resolve to the terminal id; cycles break at the first repeat.
 */
function makeCanonical(people) {
  return (id) => {
    if (!id) return id;
    let cur = id;
    const seen = new Set();
    while (people[cur] && people[cur].merged_into && !seen.has(cur)) {
      seen.add(cur);
      cur = people[cur].merged_into;
    }
    return cur;
  };
}

/**
 * Build a matcher(gitName, gitEmail) → personId|null from the people registry
 * {id: {name, aliases: []}}. Matches on normalized display name, email,
 * email local-part, or any alias; also tolerates "First Last"↔"Last First".
 */
function makeAuthorMatcher(people) {
  const index = new Map();
  const add = (k, id) => {
    const n = normName(k);
    if (n && !index.has(n)) index.set(n, id);
  };
  for (const [id, p] of Object.entries(people || {})) {
    add(p.name, id);
    const parts = normName(p.name).split(' ');
    if (parts.length === 2) add(`${parts[1]} ${parts[0]}`, id);
    for (const a of p.aliases || []) {
      add(a, id);
      if (String(a).includes('@')) add(String(a).split('@')[0], id);
    }
  }
  return (gitName, gitEmail) => {
    const email = normName(gitEmail);
    return (
      index.get(normName(gitName)) ??
      (email ? index.get(email) : undefined) ??
      (email && email.includes('@') ? index.get(email.split('@')[0]) : undefined) ??
      null
    );
  };
}

/**
 * Contributor credit split for one record: matched git authors share by commit
 * count; when nobody matches, the completion-time assignee gets full credit.
 * → [{person_id, share, commits}] (shares sum to 1 when non-empty)
 */
function contributorCredits(record, matcher) {
  const byPerson = new Map();
  let matchedCommits = 0;
  for (const a of record.git_authors || []) {
    const id = matcher ? matcher(a.name, a.email) : null;
    if (!id) continue;
    byPerson.set(id, (byPerson.get(id) || 0) + (a.commits || 0));
    matchedCommits += a.commits || 0;
  }
  if (!byPerson.size || matchedCommits <= 0) {
    return record.assignee_id ? [{ person_id: record.assignee_id, share: 1, commits: 0 }] : [];
  }
  return [...byPerson.entries()]
    .map(([person_id, commits]) => ({ person_id, share: commits / matchedCommits, commits }))
    .sort((a, b) => b.share - a.share);
}

/** Unmatched git author names across records (for the alias editor). */
function unmatchedAuthors(records, matcher) {
  const seen = new Map();
  for (const r of records) {
    for (const a of r.git_authors || []) {
      if (matcher(a.name, a.email)) continue;
      const k = `${a.name} <${a.email}>`;
      seen.set(k, (seen.get(k) || 0) + a.commits);
    }
  }
  return [...seen.entries()].map(([who, commits]) => ({ who, commits })).sort((a, b) => b.commits - a.commits);
}

// ---------------------------------------------------------------------------
// Baselines ("how long it should have taken") — effective (git-primary) days
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
    impl: stats3(recs.map((r) => rImpl(r) ?? 0)),
    total: stats3(recs.map((r) => rCycle(r))),
  };
}

const MIN_BUCKET = 3;

/**
 * Percentile buckets over completed, non-stale records, keyed (type, points),
 * with a fallback chain (type+points → type → all) and a minimum sample size.
 */
function baselines(records) {
  const completed = records.filter((r) => isDone(r) && rCycle(r) !== null && !isExcluded(r) && isTimingSample(r));
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

  // lookup() runs once per record across several stat passes — on a 13k-issue
  // corpus recomputing (and re-sorting) bucket percentiles per call turns the
  // overview into tens of seconds. Buckets are invariant per key: memoize.
  const bucketCache = new Map();
  const cached = (key, recs) => {
    let b = bucketCache.get(key);
    if (!b) {
      b = bucketOf(recs);
      bucketCache.set(key, b);
    }
    return b;
  };

  function lookup(type, points) {
    const kp = `${type}|${points ?? 'unestimated'}`;
    const tp = byTP.get(kp);
    if (tp && tp.length >= MIN_BUCKET) return { bucket: cached(`tp:${kp}`, tp), level: 'type+points' };
    const t = byT.get(type);
    if (t && t.length >= MIN_BUCKET) return { bucket: cached(`t:${type}`, t), level: 'type' };
    if (completed.length >= MIN_BUCKET) return { bucket: cached('all', completed), level: 'all' };
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
// Scope estimates (3 levels: agnostic AI → per-dev expected → actual)
// ---------------------------------------------------------------------------

/**
 * Dev-agnostic size of a record in ideal days: AI estimate when present,
 * else the record's baseline bucket p50, else the corpus median cycle, else 1.
 */
function scopeDays(record, estimates, base, fallbackMedian) {
  const est = estimates && estimates[record.key];
  if (est && typeof est.days === 'number' && est.days > 0) return est.days;
  const hit = base && base.lookup(record.type, record.points);
  if (hit && hit.bucket.total.p50) return hit.bucket.total.p50;
  return fallbackMedian || 1;
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
 * Per-assignee velocity factor: median of (actual eff cycle / bucket p50) over
 * their completed tasks; clamped [0.5, 2]; needs ≥3 samples else 1.0.
 */
function assigneeFactor(records, base) {
  const ratios = new Map();
  for (const r of records) {
    if (!isDone(r) || rCycle(r) === null || !r.assignee_id || isExcluded(r)) continue;
    const hit = base.lookup(r.type, r.points);
    if (!hit || !hit.bucket.total.p50) continue;
    if (!ratios.has(r.assignee_id)) ratios.set(r.assignee_id, []);
    ratios.get(r.assignee_id).push(rCycle(r) / hit.bucket.total.p50);
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

/**
 * Predicted timeline for a non-done record (null when no baseline exists).
 * When an AI estimate exists it becomes the p50 anchor (the bucket keeps the
 * band shape): agnostic estimate × dev factor = per-dev expected.
 */
function predict(record, base, factorMap, nowMs, workweek = WORKWEEK, estimates = null) {
  const hit = base.lookup(record.type, record.points);
  const est = estimates && estimates[record.key];
  if (!hit && !(est && est.days > 0)) return null;
  const f = record.assignee_id && factorMap.has(record.assignee_id) ? factorMap.get(record.assignee_id).factor : 1.0;
  let design = hit ? scale(hit.bucket.design, f) : { p25: null, p50: null, p75: null };
  let impl = hit ? scale(hit.bucket.impl, f) : { p25: null, p50: null, p75: null };
  let total = hit ? scale(hit.bucket.total, f) : { p25: null, p50: null, p75: null };
  if (est && typeof est.days === 'number' && est.days > 0) {
    // Re-anchor the band on the agnostic estimate, preserving relative spread.
    const anchor = round2(est.days * f);
    const spread = total.p50 ? { lo: total.p25 / total.p50, hi: total.p75 / total.p50 } : { lo: 0.7, hi: 1.4 };
    total = { p25: round2(anchor * spread.lo), p50: anchor, p75: round2(anchor * spread.hi) };
  }
  const elapsed = (record.design_days || 0) + (rImpl(record) || 0);
  const pct = total.p50 ? round2(elapsed / total.p50) : null;
  const remaining = total.p50 !== null ? Math.max(0, total.p50 - elapsed) : null;
  return {
    design,
    impl,
    total,
    factor: f,
    based_on: est && est.days > 0 ? 'ai_estimate' : hit.level,
    n: hit ? hit.bucket.n : 0,
    est_days_ai: est && est.days > 0 ? est.days : null,
    elapsed_active_days: round2(elapsed),
    pct_consumed: pct,
    projected_done_at: remaining !== null ? addBusinessDays(nowMs, remaining, workweek) : null,
  };
}

// ---------------------------------------------------------------------------
// Per-assignee stats (scope-weighted, multi-dev aware), goals
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

const MONTH_WINDOW = 12;

/** Whole months between a UTC ms timestamp and `nowMs` (0 = current month). */
function monthsAgo(ms, nowMs) {
  const a = new Date(ms);
  const b = new Date(nowMs);
  return (b.getUTCFullYear() - a.getUTCFullYear()) * 12 + (b.getUTCMonth() - a.getUTCMonth());
}

/**
 * Per-person summary stats over the corpus.
 * opts: {estimates?: {key:{days,routine}}, people?: registry, sigCounts?: Map,
 *        matcher?: fn, sinceMs?: number, untilMs?: number, canonical?: (id)=>id}
 * — people/matcher enable multi-dev credit; estimates enable scope-weighted
 * throughput + estimate accuracy; sinceMs/untilMs restrict completed SAMPLES
 * to the period (open tasks count only when untilMs is unset — historical
 * report windows have no meaningful WIP); canonical folds merged accounts.
 *
 * Sub-task rollups (isExcluded) and bulk-closed zero-time records
 * (isTimingSample) never feed medians, pace, or weighted throughput.
 */
function assigneeStats(records, base, workweek = WORKWEEK, nowMs = Date.now(), opts = {}) {
  const estimates = opts.estimates || {};
  const sinceMs = opts.sinceMs || 0;
  const untilMs = opts.untilMs || 0;
  const canonical = opts.canonical || ((id) => id);
  const sigCounts = opts.sigCounts || routineSignatures(records);
  const matcher = opts.matcher || (opts.people ? makeAuthorMatcher(opts.people) : null);
  const factorMap = assigneeFactor(records, base);

  const completedAll = records.filter((r) => isDone(r) && rCycle(r) !== null && !isExcluded(r) && isTimingSample(r));
  const fallbackMedian = median(completedAll.map((r) => rCycle(r)));
  const allPaces = []; // actual ÷ scope-estimate across every included sample

  const byDev = new Map();
  const dev = (id, name) => {
    if (!byDev.has(id)) {
      byDev.set(id, {
        name: name || null,
        completed: [], open: [],
        weighted_done: 0, weighted_share_n: 0,
        contributed: 0, shared: 0, rolled_up_n: 0,
        routine_done: 0, feature_done: 0,
        // Size-weighted pace: Σactual vs Σestimate (share-credited) — a pile of
        // accurate 0.5d tasks cannot drown one 5d task that took 12.
        sum_actual: 0, sum_est: 0, pace_n: 0,
        monthly: new Array(MONTH_WINDOW).fill(0), // weighted est-days, oldest→current month
        monthly_est: new Array(MONTH_WINDOW).fill(0), // Σ AI-estimate (ratio basis) per month
        monthly_actual: new Array(MONTH_WINDOW).fill(0), // Σ actual days per month
        first_credit: null,
      });
    }
    const g = byDev.get(id);
    if (!g.name && name) g.name = name;
    return g;
  };

  for (const r of records) {
    // Lead-excluded stories vanish from the stats entirely (still listed in
    // task views for re-inclusion).
    if (r.excluded_override === true) continue;
    const aid = canonical(r.assignee_id);
    if (aid) {
      const g = dev(aid, r.assignee_name);
      if (isDone(r)) {
        if (inPeriod(r, sinceMs, untilMs)) {
          if (r.rollup === true) g.rolled_up_n++;
          else g.completed.push(r);
        }
      } else if (!untilMs) g.open.push(r);
    }
    // Multi-dev credit: contributors get weighted-throughput + efficiency
    // credit by commit share, whoever the Jira assignee was.
    if (isDone(r) && inPeriod(r, sinceMs, untilMs)) {
      const credits = contributorCredits(r, matcher).map((c) => ({ ...c, person_id: canonical(c.person_id) }));
      const sd = scopeDays(r, estimates, base, fallbackMedian);
      // Ratios (pace / efficiency) demand a uniform basis: only tasks with a
      // real AI estimate — bucket fallbacks are elapsed-day medians and mixing
      // them skews devs unevenly by estimate coverage.
      const estAI = estimates[r.key] && estimates[r.key].days > 0 ? estimates[r.key].days : null;
      const actual = rCycle(r);
      const routine = isRoutine(r, sigCounts, estimates[r.key]);
      const measurable = !isExcluded(r) && isTimingSample(r);
      const mi = monthsAgo(rDoneAt(r) ?? nowMs, nowMs);
      for (const c of credits) {
        const g = dev(c.person_id, c.person_id === aid ? r.assignee_name : null);
        if (!isExcluded(r)) {
          g.weighted_done += sd * c.share;
          g.weighted_share_n += c.share;
          if (mi >= 0 && mi < MONTH_WINDOW) g.monthly[MONTH_WINDOW - 1 - mi] += sd * c.share;
          if (measurable && actual !== null && actual > 0 && estAI !== null) {
            g.sum_actual += actual * c.share;
            g.sum_est += estAI * c.share;
            g.pace_n++;
            allPaces.push([actual * c.share, estAI * c.share]);
            if (mi >= 0 && mi < MONTH_WINDOW) {
              g.monthly_est[MONTH_WINDOW - 1 - mi] += estAI * c.share;
              g.monthly_actual[MONTH_WINDOW - 1 - mi] += actual * c.share;
            }
          }
          if (!isExcluded(r)) {
            const dAt = rDoneAt(r);
            if (dAt && (g.first_credit === null || dAt < g.first_credit)) g.first_credit = dAt;
          }
        }
        if (routine) g.routine_done += c.share;
        else g.feature_done += c.share;
        if (c.person_id !== aid) g.contributed++;
        if ((r.flags || []).includes('multi_dev')) g.shared++;
      }
    }
  }

  const WEEK = 7 * DAY;
  const periodWeeks = untilMs && sinceMs
    ? Math.max(1, (untilMs - sinceMs) / WEEK)
    : sinceMs
      ? Math.max(1, (nowMs - sinceMs) / WEEK)
      : null;

  // Team pace = total actual days ÷ total estimated days across the scope —
  // size-weighted, same construction as the per-dev pace it normalizes.
  const teamActual = allPaces.reduce((a, [ac]) => a + ac, 0);
  const teamEst = allPaces.reduce((a, [, e]) => a + e, 0);
  const teamPace = teamEst > 0 ? teamActual / teamEst : null;
  const out = [];
  for (const [id, g] of byDev) {
    const done = g.completed
      .filter((r) => rCycle(r) !== null && !isExcluded(r) && isTimingSample(r))
      .sort((a, b) => rDoneAt(a) - rDoneAt(b));
    const cycles = done.map((r) => rCycle(r));
    // Estimate accuracy vs the dev-agnostic estimate (AI first, Jira estimate fallback).
    const errs = done
      .map((r) => {
        const est = estimates[r.key];
        const e = est && est.days > 0 ? est.days : r.estimate_days;
        return e && e > 0 ? Math.abs(rCycle(r) - e) / e : null;
      })
      .filter((v) => v !== null);
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
      ...done.map((r) => ({ from: r.first_active_at ?? r.first_commit_at, to: rDoneAt(r) })),
      ...g.open.filter((r) => r.first_active_at || r.first_commit_at).map((r) => ({ from: r.first_active_at ?? r.first_commit_at, to: null })),
    ];
    const flags = {};
    for (const r of g.completed) for (const fl of r.flags || []) flags[fl] = (flags[fl] || 0) + 1;
    const devPace = g.sum_est > 0 ? g.sum_actual / g.sum_est : null;
    // Delivered est-days per calendar week: parallel work can't inflate this —
    // the calendar is the denominator, not summed per-task elapsed time.
    const weeks = periodWeeks ?? (g.first_credit ? Math.max(1, (nowMs - g.first_credit) / WEEK) : null);
    out.push({
      output_wk: weeks && g.weighted_done > 0 ? round2(g.weighted_done / weeks) : null,
      monthly_est: g.monthly_est.map((v) => Math.round(v * 10) / 10),
      monthly_actual: g.monthly_actual.map((v) => Math.round(v * 10) / 10),
      assignee_id: id,
      assignee_name: g.name || id,
      completed: g.completed.length,
      rolled_up: g.rolled_up_n,
      stale: g.completed.filter((r) => isStale(r)).length,
      outliers: g.completed.filter((r) => r.outlier === true).length,
      wip: g.open.filter((r) => r.first_active_at !== null || r.first_commit_at).length,
      open: g.open.length,
      median_design: median(done.map((r) => rDesign(r)).filter((v) => v !== null)),
      median_impl: median(done.map((r) => rImpl(r)).filter((v) => v !== null)),
      // Fix/deploy medians treat "no fixes"/"not deployed yet" as 0 so the
      // chart shows the typical task, not the typical *fixed* task.
      median_fix: median(done.map((r) => r.fix_days ?? 0)),
      median_deploy: median(done.map((r) => r.deploy_wait_days ?? 0)),
      median_cycle: median(cycles),
      factor: factorMap.has(id) ? factorMap.get(id).factor : null,
      // Absolute pace: Σactual ÷ Σ(AI estimate) — ×4.2 means the work takes
      // 4.2 elapsed days per estimated ideal day. This is the number a lead
      // recognizes from reading tasks.
      pace_vs_est: devPace !== null && g.pace_n >= 3 ? round2(devPace) : null,
      // vs team: the same ratio normalized by the team's pooled ratio (removes
      // the systemic ideal-vs-elapsed gap). ×1.6 = 60% slower than team pace.
      pace_factor: devPace !== null && g.pace_n >= 3 && teamPace > 0 ? round2(devPace / teamPace) : null,
      mape: errs.length ? round2(median(errs)) : null,
      avg_wip: avgWip(windows, workweek, nowMs),
      weighted_done: round2(g.weighted_done),
      // Size-weighted sums behind pace/efficiency — the chart modes plot them.
      sum_actual: round2(g.sum_actual),
      sum_est: round2(g.sum_est),
      sum_est_dev: round2(g.sum_est * (factorMap.has(id) ? factorMap.get(id).factor : 1)),
      // Efficiency is the size-weighted inverse pace: estimated days delivered
      // per actual day spent (>1 = faster than the estimates).
      efficiency: g.sum_actual > 0 && g.sum_est > 0 ? round2(g.sum_est / g.sum_actual) : null,
      routine_done: Math.round(g.routine_done * 10) / 10,
      feature_done: Math.round(g.feature_done * 10) / 10,
      contributed: g.contributed,
      shared: g.shared,
      monthly: g.monthly.map((v) => Math.round(v * 10) / 10),
      flags,
      trend,
    });
  }
  // vs team (output): each dev's weekly output relative to the team median —
  // higher = more delivered scope per week. This is the volume-fair companion
  // to pace_factor (which measures per-task latency instead).
  const teamOut = median(out.map((s) => s.output_wk).filter((v) => v !== null && v > 0));
  for (const s of out) {
    s.output_factor = s.output_wk !== null && teamOut > 0 ? round2(s.output_wk / teamOut) : null;
  }
  out.sort((a, b) => (b.weighted_done || 0) - (a.weighted_done || 0) || b.completed - a.completed || String(a.assignee_name).localeCompare(String(b.assignee_name)));
  return out;
}

const GOAL_METRICS = ['median_cycle_days', 'median_impl_days', 'median_design_days', 'estimate_mape', 'avg_wip', 'weighted_throughput', 'efficiency'];

// stats-field name behind each goal metric + direction (down = lower is better).
const GOAL_DEFS = {
  median_cycle_days: { src: 'median_cycle', dir: 'down' },
  median_impl_days: { src: 'median_impl', dir: 'down' },
  median_design_days: { src: 'median_design', dir: 'down' },
  estimate_mape: { src: 'mape', dir: 'down' },
  avg_wip: { src: 'avg_wip', dir: 'down' },
  weighted_throughput: { src: 'weighted_done', dir: 'up' },
  efficiency: { src: 'efficiency', dir: 'up' },
};

/**
 * Suggested targets: step 10% toward the team median, never suggesting
 * regression for a dev already better than the team. Directional.
 */
function suggestGoals(devStats, teamMedians) {
  const out = [];
  for (const metric of GOAL_METRICS) {
    const def = GOAL_DEFS[metric];
    const own = devStats[def.src] ?? devStats[metric] ?? null;
    const team = teamMedians[metric] ?? null;
    if (own === null || team === null) continue;
    const target = def.dir === 'down'
      ? round2(Math.min(own, Math.max(team, own * 0.9)))
      : round2(Math.max(own, Math.min(team, own * 1.1)));
    if (target <= 0) continue;
    out.push({ metric, target });
  }
  return out;
}

/** Progress of one goal against current stats (direction-aware). */
function goalProgress(goal, stats) {
  const def = GOAL_DEFS[goal.metric] || { src: goal.metric, dir: 'down' };
  const current = stats[def.src] ?? stats[goal.metric] ?? null;
  const met = current !== null && (def.dir === 'down' ? current <= goal.target : current >= goal.target);
  return { current, met, dir: def.dir };
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
    weighted_throughput: m('weighted_done'),
    efficiency: m('efficiency'),
  };
}

// ---------------------------------------------------------------------------
// Scope (team / role) metrics + goals
// ---------------------------------------------------------------------------

const SCOPE_METRICS = ['median_cycle_days', 'median_impl_days', 'median_deploy_lead_days', 'fix_rate', 'on_track_pct', 'weighted_throughput_wk'];

const SCOPE_DEFS = {
  median_cycle_days: { dir: 'down', fmt: 'days' },
  median_impl_days: { dir: 'down', fmt: 'days' },
  median_deploy_lead_days: { dir: 'down', fmt: 'days' },
  fix_rate: { dir: 'down', fmt: 'pct' },
  on_track_pct: { dir: 'up', fmt: 'pct' },
  weighted_throughput_wk: { dir: 'up', fmt: 'days' },
};

/**
 * Task-level metrics over a set of records (a whole scope or one role's
 * records), optionally restricted to completions after `sinceMs`. Weighted
 * throughput is per week over the trailing 8 weeks.
 */
function scopeMetrics(records, base, workweek = WORKWEEK, nowMs = Date.now(), estimates = {}, sinceMs = 0) {
  const done = records.filter((r) => isDone(r) && rCycle(r) !== null && !isExcluded(r) && isTimingSample(r) && inPeriod(r, sinceMs));
  const delivered = records.filter((r) => r.done_git_at && !r.rollup && inPeriod(r, sinceMs));
  const fixed = delivered.filter((r) => (r.fix_count || 0) > 0);
  const deployLeads = delivered
    .filter((r) => r.deployed_at && r.deployed_at > r.done_git_at)
    .map((r) => r.deploy_wait_days)
    .filter((v) => v !== null);
  const onTrack = done.filter((r) => {
    const hit = base.lookup(r.type, r.points);
    const v = hit ? verdict(rCycle(r), hit.bucket.total.p50) : null;
    return v === 'fast' || v === 'on_track';
  });
  const fallbackMedian = median(done.map((r) => rCycle(r)));
  const cutoff = nowMs - 56 * DAY;
  const recentDone = done.filter((r) => rDoneAt(r) >= cutoff);
  const weighted = recentDone.reduce((acc, r) => acc + scopeDays(r, estimates, base, fallbackMedian), 0);
  return {
    median_cycle_days: median(done.map((r) => rCycle(r))),
    median_impl_days: median(done.map((r) => rImpl(r)).filter((v) => v !== null)),
    median_deploy_lead_days: deployLeads.length ? median(deployLeads) : null,
    fix_rate: delivered.length ? round2(fixed.length / delivered.length) : null,
    on_track_pct: done.length ? round2(onTrack.length / done.length) : null,
    weighted_throughput_wk: round2(weighted / 8),
  };
}

/** Evaluate configured scope goals [{metric,target}] against metric values. */
function evalScopeGoals(goals, values) {
  const out = [];
  for (const g of goals || []) {
    const def = SCOPE_DEFS[g.metric];
    if (!def) continue;
    const current = values[g.metric] ?? null;
    out.push({
      metric: g.metric,
      target: g.target,
      current,
      dir: def.dir,
      met: current !== null && (def.dir === 'down' ? current <= g.target : current >= g.target),
    });
  }
  return out;
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
  deriveGit,
  isDone,
  isExcluded,
  isTimingSample,
  actualDays: rCycle, // the canonical actual (manual > cycle + folded fixes)
  enrichHierarchy,
  makeCanonical,
  effDoneAt,
  baselines,
  verdict,
  assigneeFactor,
  predict,
  assigneeStats,
  suggestGoals,
  goalProgress,
  teamMedians,
  scopeMetrics,
  evalScopeGoals,
  scopeDays,
  routineSignature,
  routineSignatures,
  isRoutine,
  makeAuthorMatcher,
  contributorCredits,
  unmatchedAuthors,
  median,
  GOAL_METRICS,
  GOAL_DEFS,
  SCOPE_METRICS,
  SCOPE_DEFS,
};
