// DORA metrics — pure functions, no I/O.
//
// Inputs are what the scan already persists:
//   records: issue records ({key, type, subtask?, created, first_commit_at,
//            deployed_at, deployed_tag?, rework_of?, origin?, rework_signals?,
//            rework_confidence?, rework_self?, reopened_at?})
//   tags:    deploy tags from gitscan ({repo, name, ts, sha?, contains?}).
//            Which tags count and their kind come from gitscan's isDeployTag /
//            tagKind — never a local regex — so the scan and DORA agree.
//   config:  {timezone, weekend, failure_window_days, min_n, window:{start,end},
//            prs, deploy_tag_patterns, hotfix_tag_patterns, tagKind(name, cfg),
//            branch, repos, tagContains(tag, sha), capacity_days}
//
// Every share / rate below is a 0..1 FRACTION (never a percentage).
//
// Definitions (documented so a number is never misread):
//   deploy_frequency    distinct deploy events in the window, per week. An
//                       event = one (repo, tagged sha): several tags on the same
//                       commit in one repo = 1 deploy; different commits are
//                       different deploys even on the same day. A tag with no
//                       repo (or no sha) is its own event per tag name.
//                       Kind 'hotfix' when any of its tag names contains one of
//                       hotfix_tag_patterns (default hf / hotfix), or per the
//                       caller's tagKind(name, cfg) callback.
//   deploys_per_capacity_week  deploys / (team capacity_days / 5) — present
//                       only when the caller passes capacity_days (vacations
//                       already removed), so frequency is read against people.
//   hotfix_rate         hotfix deploys / all deploys in the window.
//   batch_size          tickets shipped per deploy event (P50 / P85).
//   time_to_first_commit business days ticket start (first_active_at) → first
//                       commit, P50 over tickets deployed in the window.
//   lead_time           PRIMARY: per merged PR, business days (team timezone)
//                       first commit → first deploy tag containing the merge
//                       sha (else the linked ticket's deployed_at). Secondary
//                       ticket_lead_time: first_commit_at → deployed_at over
//                       tickets (sub-tasks, epics, roll-ups, excluded skipped).
//   change_failure_rate failed deploys / deploys ("N of M"). A deploy FAILED
//                       when, within failure_window_days after it (inclusive),
//                       the SAME repo got a hotfix tag, or a Bug whose
//                       rework_of/origin points at a key it shipped was
//                       created, or a Jira-detected rework (link / title /
//                       reopened) points at such a key. Chained hotfixes inside
//                       the window are ONE incident.
//   mttr                per incident: detection (bug created) — else the deploy
//                       itself, flagged from_deploy — → restore (the bug's own
//                       deploy in the same repo, else the last chained hotfix).
//                       Split into time_to_detect (deploy → detection, only for
//                       detected incidents) and time_to_restore (detection →
//                       restore); from_deploy_share = from_deploy / restored.
//   open_incidents      window incidents with no restore yet: {n, oldest_age}
//                       (oldest_age in calendar days, deploy → window end).
// Every metric is {value:null} (never 0 / NaN) when its denominator is empty,
// and gets a canonical guardrail (id = tile key) when its sample is weak.
'use strict';

const A = require('./analytics.js');
const tz = require('./tz.js');
const G = require('./gitscan.js');

const DAY = 86400000;
const HOUR = 3600000;
const DEFAULT_HOTFIX_PATTERNS = ['hf', 'hotfix'];

/** Hotfix tag patterns from config (case-insensitive substrings). */
function hotfixPatterns(cfg = {}) {
  const p = Array.isArray(cfg.hotfix_tag_patterns) ? cfg.hotfix_tag_patterns.map((x) => String(x || '').trim().toLowerCase()).filter(Boolean) : [];
  return p.length ? p : DEFAULT_HOTFIX_PATTERNS;
}

/** 'hotfix' | 'regular' for a deploy tag: caller's tagKind(name, cfg) wins,
 *  else a case-insensitive substring match on hotfix_tag_patterns. */
function deployKind(name, cfg = {}, patterns = hotfixPatterns(cfg)) {
  if (typeof cfg.tagKind === 'function') return cfg.tagKind(name, cfg) === 'hotfix' ? 'hotfix' : 'regular';
  const n = String(name || '').toLowerCase();
  return patterns.some((p) => n.includes(p)) ? 'hotfix' : 'regular';
}

/** Index of the first element of sorted `arr` with ts >= t (arr.length when none). */
function lowerBound(arr, t) {
  let lo = 0;
  let hi = arr.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (arr[mid].ts < t) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

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
  // lead time for changes, business days: elite ≤ 1 day, high ≤ 1 week,
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
  // time to restore, hours: elite ≤ 1h, high ≤ 1 day, medium ≤ 1 week.
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

const toMs = (v) => {
  if (v == null || v === '') return null;
  const t = typeof v === 'number' ? v : Date.parse(v);
  return Number.isFinite(t) ? t : null;
};
const inWindow = (t, w) => typeof t === 'number' && t >= w.start && t < w.end;
const isBug = (r) => /bug|defect|incident/i.test(String(r.type || ''));
const isSubtask = (r) => r.subtask === true || /sub-?task/i.test(String(r.type || ''));
const refsOf = (r) => {
  const out = [];
  for (const f of [r.rework_of, r.origin]) {
    if (Array.isArray(f)) out.push(...f);
    else if (f && typeof f === 'object' && f.key) out.push(f.key);
    else if (typeof f === 'string' && f) out.push(f);
  }
  return [...new Set(out)];
};
const has = (coll, v) => (coll instanceof Set ? coll.has(v) : Array.isArray(coll) ? coll.includes(v) : false);

/** Lead-time sample basis: excludes sub-tasks, epics, roll-ups and lead-excluded records. */
const leadEligible = (r) => !isSubtask(r) && !A.isExcluded(r);

const dist = (vals) => {
  const p50 = percentile(vals, 0.5);
  return { value: p50, p50, p75: percentile(vals, 0.75), p85: percentile(vals, 0.85), n: vals.length, unit: 'business_days', band: bandFor('lead_time_days', p50) };
};

/**
 * doraMetrics(records, tags, config)
 * Legacy object form doraMetrics({records, tags, window, cfg}) is still accepted.
 * → {deploy_frequency, lead_time, ticket_lead_time, change_failure_rate, mttr,
 *    incidents, guardrails, bands, window}
 */
function doraMetrics(records, tags, config) {
  if (records && !Array.isArray(records) && typeof records === 'object') {
    const o = records;
    return doraMetrics(o.records || [], o.tags || [], { ...(o.cfg || {}), ...(o.config || {}), window: o.window || (o.config && o.config.window) });
  }
  records = Array.isArray(records) ? records : [];
  tags = Array.isArray(tags) ? tags : [];
  const cfg = config || {};
  const zone = cfg.timezone || 'UTC';
  const weekend = Array.isArray(cfg.weekend) ? cfg.weekend : tz.DEFAULT_WEEKEND;
  const failWinMs = (Number(cfg.failure_window_days) > 0 ? Number(cfg.failure_window_days) : 7) * DAY;
  const minN = Number(cfg.min_n) > 0 ? Number(cfg.min_n) : 3;
  const patterns = G.deployTagPatterns(cfg);
  const bdays = (a, b) => round2(tz.businessDaysTz(a, b, { tz: zone, weekend }));

  // ---- deploy events -------------------------------------------------------
  const deployTagsAll = tags
    .map((t) => ({ ...t, ts: toMs(t.ts) }))
    .filter((t) => Number.isFinite(t.ts) && G.isDeployTag(t.name, patterns))
    .sort((a, b) => a.ts - b.ts);
  const allTs = deployTagsAll.map((t) => t.ts);
  const win = cfg.window || {};
  const w = {
    start: Number.isFinite(win.start) ? win.start : allTs.length ? allTs[0] : 0,
    end: Number.isFinite(win.end) ? win.end : allTs.length ? allTs[allTs.length - 1] + 1 : 0,
  };

  // Dedupe by repo + tagged sha: several tags on one commit = one deploy.
  // A null repo or a missing sha can't be matched safely → its own event per
  // tag name. Kind upgrades to hotfix when any tag of the event is one.
  const hfPatterns = hotfixPatterns(cfg);
  const events = new Map();
  const eventOfTag = new Map(); // `${repo}|${name}` -> event
  for (const t of deployTagsAll) {
    const repo = t.repo || null;
    const kind = deployKind(t.name, cfg, hfPatterns);
    const day = tz.dayKey(t.ts, zone);
    const id = repo && t.sha ? `${repo}|sha:${t.sha}` : `${repo || ''}|name:${t.name}`;
    let e = events.get(id);
    if (!e) {
      e = { id, repo, kind, day, ts: t.ts, tags: [], shas: [], contains: [] };
      events.set(id, e);
    } else if (kind === 'hotfix') e.kind = 'hotfix';
    e.tags.push(t.name);
    if (t.sha) e.shas.push(t.sha);
    if (t.contains) e.contains.push(t.contains);
    e.raw = e.raw || [];
    e.raw.push(t);
    eventOfTag.set(`${repo || ''}|${t.name}`, e);
  }
  const deployList = [...events.values()].sort((a, b) => a.ts - b.ts);
  const windowDeploys = deployList.filter((d) => inWindow(d.ts, w));
  const total = windowDeploys.length;

  const guardrails = [];
  const weak = (id, n, what) => {
    if (n < minN) guardrails.push({ id, metric: id, code: 'low_n', severity: 'weak', n, min_n: minN, reason: `${what}: n=${n} < ${minN}; treat as indicative only` });
  };

  // ---- deploy frequency ----------------------------------------------------
  const weeks = (w.end - w.start) / (7 * DAY);
  const byRepo = {};
  const byKind = { regular: 0, hotfix: 0 };
  for (const d of windowDeploys) {
    byRepo[d.repo || 'unknown'] = (byRepo[d.repo || 'unknown'] || 0) + 1;
    byKind[d.kind] = (byKind[d.kind] || 0) + 1;
  }
  let deploy_frequency;
  if (total === 0) {
    deploy_frequency = {
      value: null, per_week: null, total: 0, by_repo: {}, per_week_by_repo: {}, by_kind: byKind, band: null,
      status: 'not_available', reason: 'no_deploy_tags', patterns,
      repos: Array.isArray(cfg.repos) ? cfg.repos : [...new Set(tags.map((t) => t.repo).filter(Boolean))],
      branch: cfg.branch || null,
    };
    guardrails.push({ id: 'dora_deploy_frequency', metric: 'dora_deploy_frequency', code: 'not_available', severity: 'not_available', n: 0, min_n: minN, reason: `No deploy tags matching ${patterns.join(' / ')} in the period — deployment metrics are not tracked.` });
  } else {
    const perWeek = weeks > 0 ? round2(total / weeks) : null;
    const perWeekByRepo = {};
    for (const [r, n] of Object.entries(byRepo)) perWeekByRepo[r] = weeks > 0 ? round2(n / weeks) : null;
    deploy_frequency = { value: perWeek, per_week: perWeek, total, by_repo: byRepo, per_week_by_repo: perWeekByRepo, by_kind: byKind, band: bandFor('deploy_frequency', perWeek), status: 'ok' };
    const capWeeks = Number(cfg.capacity_days) > 0 ? Number(cfg.capacity_days) / 5 : null;
    deploy_frequency.per_capacity_week = capWeeks ? round2(total / capWeeks) : null;
    if (weeks <= 0) guardrails.push({ id: 'dora_deploy_frequency', metric: 'dora_deploy_frequency', code: 'empty_window', severity: 'weak', n: total, min_n: minN, reason: 'window has no length' });
    else weak('dora_deploy_frequency', total, 'deploys in window');
  }

  // ---- which deploy shipped each record ------------------------------------
  const eventForRecord = (r) => {
    const at = toMs(r.deployed_at);
    if (!Number.isFinite(at)) return null;
    if (r.deployed_tag) {
      const cand = deployList.filter((e) => e.tags.includes(r.deployed_tag));
      if (cand.length === 1) return cand[0];
      if (cand.length > 1) return cand.find((e) => e.raw.some((t) => t.ts === at)) || cand[0];
    }
    return deployList.find((e) => e.raw.some((t) => t.ts === at)) || null;
  };
  const recEvent = new Map();
  const shippedBy = new Map(); // key -> event
  for (const r of records) {
    const e = eventForRecord(r);
    recEvent.set(r, e);
    if (e && r.key) shippedBy.set(r.key, e);
  }

  // ---- lead time -----------------------------------------------------------
  const ticketLeads = [];
  for (const r of records) {
    const dep = toMs(r.deployed_at);
    const fc = toMs(r.first_commit_at);
    if (!leadEligible(r) || !inWindow(dep, w) || !Number.isFinite(fc) || fc > dep) continue;
    ticketLeads.push(bdays(fc, dep));
  }
  const ticket_lead_time = { ...dist(ticketLeads), basis: 'ticket' };

  const byKey = new Map(records.filter((r) => r.key).map((r) => [r.key, r]));
  // sha → earliest deploy tag containing it, per repo and repo-agnostic
  // (built once from tag.sha + tag.contains, tags already ts-sorted → first
  // write wins). O(1) per PR; a caller tagContains() callback is the
  // fallback, binary-searched to the first tag at/after the merge.
  const shaFirst = new Map(); // `${repo}|${sha}` and `*|${sha}` -> tag
  const tagsByRepo = new Map(); // repo -> ts-sorted tags
  for (const t of deployTagsAll) {
    const repo = t.repo || '';
    if (!tagsByRepo.has(repo)) tagsByRepo.set(repo, []);
    tagsByRepo.get(repo).push(t);
    const shas = [t.sha, ...(Array.isArray(t.contains) ? t.contains : t.contains instanceof Set ? [...t.contains] : [])];
    for (const s of shas) {
      if (!s) continue;
      if (!shaFirst.has(`${repo}|${s}`)) shaFirst.set(`${repo}|${s}`, t);
      if (!shaFirst.has(`*|${s}`)) shaFirst.set(`*|${s}`, t);
    }
  }
  const findDeployTag = (repo, sha, merged) => {
    const hit = shaFirst.get(repo ? `${repo}|${sha}` : `*|${sha}`) || (repo ? shaFirst.get(`|${sha}`) : null);
    if (hit && hit.ts >= merged) return hit;
    if (typeof cfg.tagContains !== 'function') return null;
    const pools = repo ? [tagsByRepo.get(repo) || [], tagsByRepo.get('') || []] : [deployTagsAll];
    let best = null;
    for (const pool of pools) {
      for (let i = lowerBound(pool, merged); i < pool.length; i++) {
        if (best && pool[i].ts >= best.ts) break;
        if (cfg.tagContains(pool[i], sha)) { best = pool[i]; break; }
      }
    }
    return best;
  };
  const prLeads = [];
  const prSources = { tag_contains_merge: 0, ticket_deployed_at: 0, considered: 0, tag_contains_merge_coverage: null };
  for (const p of Array.isArray(cfg.prs) ? cfg.prs : []) {
    const merged = toMs(p && p.merged_at);
    if (!Number.isFinite(merged)) continue;
    const start = toMs(p.first_commit_at) ?? toMs(p.opened_at);
    if (!Number.isFinite(start)) continue;
    prSources.considered++;
    let dep = null;
    const sha = p.merge_commit || p.head_sha;
    if (sha) {
      const t = findDeployTag(p.repo || null, sha, merged);
      if (t) { dep = t.ts; prSources.tag_contains_merge++; }
    }
    if (dep === null) {
      const keyed = (p.keys || []).map((k) => byKey.get(k)).filter((r) => r && leadEligible(r));
      const ds = keyed.map((r) => toMs(r.deployed_at)).filter((t) => Number.isFinite(t) && t >= merged);
      if (ds.length) { dep = Math.min(...ds); prSources.ticket_deployed_at++; }
    }
    if (dep === null || !inWindow(dep, w) || start > dep) continue;
    prLeads.push(bdays(start, dep));
  }
  prSources.tag_contains_merge_coverage = prSources.considered ? round2(prSources.tag_contains_merge / prSources.considered) : null;
  const lead_time = prLeads.length
    ? { ...dist(prLeads), basis: 'pr', sources: prSources }
    : { ...ticket_lead_time, basis: 'ticket', fallback_reason: 'no_merged_prs_with_deploy' };
  weak('dora_lead_time', lead_time.n, lead_time.basis === 'pr' ? 'merged PRs with a deploy' : 'deployed tickets with a first commit');

  // ---- incidents (CFR + MTTR) ----------------------------------------------
  // Latest deploy in `repo` strictly before `t` within the failure window.
  const culpritFor = (repo, t, exclude) => {
    for (let i = deployList.length - 1; i >= 0; i--) {
      const d = deployList[i];
      if (d.ts >= t) continue;
      if (t - d.ts > failWinMs) return null;
      if (d.repo === repo && d !== exclude) return d;
    }
    return null;
  };
  const incidents = new Map(); // culprit event id -> incident
  const incidentOf = (d) => {
    let inc = incidents.get(d.id);
    if (!inc) {
      inc = { deploy: d, repo: d.repo, signals: [], hotfixes: [] };
      incidents.set(d.id, inc);
    }
    return inc;
  };
  const chainOf = new Map(); // hotfix event id -> incident it restores
  for (const h of deployList.filter((e) => e.kind === 'hotfix')) {
    let d = culpritFor(h.repo, h.ts, h);
    // Chained hotfix: the culprit is itself a hotfix already restoring an
    // incident → the same incident, not a new failure of that hotfix.
    if (d && chainOf.has(d.id)) {
      const inc = chainOf.get(d.id);
      inc.hotfixes.push(h);
      inc.signals.push({ kind: 'hotfix_tag', at: h.ts, ref: h.tags[0], confidence: 'high', chained: true });
      chainOf.set(h.id, inc);
      continue;
    }
    if (!d) continue;
    const inc = incidentOf(d);
    inc.hotfixes.push(h);
    inc.signals.push({ kind: 'hotfix_tag', at: h.ts, ref: h.tags[0], confidence: 'high' });
    chainOf.set(h.id, inc);
  }

  const linkedBugs = new Set();
  for (const r of records) {
    const created = toMs(r.created ?? r.created_at);
    const bug = isBug(r);
    const refs = refsOf(r);
    const jiraSignals = Array.isArray(r.rework_signals) ? r.rework_signals.filter((s) => s !== 'blame') : [];
    if (r.rework_self && jiraSignals.includes('reopened')) {
      const e = shippedBy.get(r.key);
      const at = toMs(r.reopened_at);
      if (e && Number.isFinite(at) && at > e.ts && at - e.ts <= failWinMs) {
        incidentOf(e).signals.push({ kind: 'jira_reopened', at, ref: r.key, of: r.key, confidence: r.rework_confidence || 'medium' });
      }
      continue;
    }
    if (!Number.isFinite(created) || !refs.length) continue;
    for (const k of refs) {
      const e = shippedBy.get(k);
      if (!e || !(created > e.ts) || created - e.ts > failWinMs) continue;
      if (bug) {
        linkedBugs.add(r.key);
        incidentOf(e).signals.push({ kind: 'bug', at: created, ref: r.key, of: k, confidence: 'high', record: r });
      } else if (jiraSignals.length) {
        incidentOf(e).signals.push({ kind: 'jira_rework', at: created, ref: r.key, of: k, confidence: r.rework_confidence || 'low', via: jiraSignals });
      }
      break;
    }
  }

  const windowIds = new Set(windowDeploys.map((d) => d.id));
  const winIncidents = [...incidents.values()].filter((i) => windowIds.has(i.deploy.id)).sort((a, b) => a.deploy.ts - b.deploy.ts);
  const bySignal = { hotfix_tag: 0, bug: 0, jira_rework: 0, jira_reopened: 0 };
  const evidence = [];
  const restores = [];
  const mttrEvidence = [];
  const failures = [];
  const detects = [];
  const restoresFromDetect = [];
  const openInc = [];
  for (const inc of winIncidents) {
    inc.signals.sort((a, b) => a.at - b.at);
    const kinds = new Set(inc.signals.map((s) => s.kind));
    for (const k of kinds) bySignal[k] = (bySignal[k] || 0) + 1;
    const conf = inc.signals.some((s) => s.confidence === 'high') ? 'high' : inc.signals.some((s) => s.confidence === 'medium') ? 'medium' : 'low';
    const d = inc.deploy;
    evidence.push({
      deploy: d.tags[0], repo: d.repo, at: d.ts, confidence: conf,
      signals: inc.signals.map(({ record, ...s }) => s),
    });

    // MTTR: detection = earliest linked bug's creation; else the deploy itself.
    const bugSig = inc.signals.find((s) => s.kind === 'bug');
    const from = bugSig ? bugSig.at : d.ts;
    const fromSource = bugSig ? 'detected' : 'from_deploy';
    let to = null;
    let restoredBy = null;
    if (bugSig) {
      const be = recEvent.get(bugSig.record);
      if (be && be.repo === d.repo && be.ts > from) { to = be.ts; restoredBy = be.tags[0]; }
    }
    if (to === null && inc.hotfixes.length) {
      const last = inc.hotfixes[inc.hotfixes.length - 1];
      if (last.repo === d.repo && last.ts > from) { to = last.ts; restoredBy = last.tags[0]; }
    }
    const hours = to !== null ? round2((to - from) / HOUR) : null;
    if (bugSig) detects.push(round2((bugSig.at - d.ts) / HOUR));
    if (hours === null) openInc.push(d);
    if (hours !== null) {
      restores.push(hours);
      if (bugSig) restoresFromDetect.push(hours);
      mttrEvidence.push({ deploy: d.tags[0], repo: d.repo, from, from_source: fromSource, from_deploy: fromSource === 'from_deploy', restored_at: to, restored_by: restoredBy, hours, signal: (bugSig || inc.signals[0]).ref });
    }
    const primary = bugSig || inc.signals[0];
    failures.push({ key: primary.kind === 'hotfix_tag' ? '' : primary.ref, tag: d.tags[0], kind: primary.kind, caused_by: primary.of || '', repo: d.repo, confidence: conf, restore_hours: hours });
  }

  const failed = winIncidents.length;
  const cfr = total > 0 ? round2(failed / total) : null;
  const change_failure_rate = {
    value: cfr, failed, total, label: total > 0 ? `${failed} of ${total}` : null,
    by_signal: bySignal, evidence, failures, band: bandFor('change_failure_rate', cfr),
    window_days: failWinMs / DAY,
    ...(total === 0 ? { status: 'not_available', reason: 'no_deploy_tags' } : {}),
  };
  if (total > 0) weak('dora_cfr', total, 'deploys in window');

  const mttrP50 = percentile(restores, 0.5);
  const mttr = {
    value: mttrP50, p50: mttrP50, p75: percentile(restores, 0.75), p85: percentile(restores, 0.85),
    unit: 'hours', n: restores.length, incidents: failed,
    from_deploy: mttrEvidence.filter((e) => e.from_deploy).length,
    evidence: mttrEvidence, band: bandFor('mttr_hours', mttrP50),
    from_deploy_share: restores.length ? round2(mttrEvidence.filter((e) => e.from_deploy).length / restores.length) : null,
    // Detection vs repair: deploy → bug created, and bug created → restore
    // (detected incidents only; from_deploy incidents have no detection time).
    time_to_detect: { p50: percentile(detects, 0.5), p85: percentile(detects, 0.85), n: detects.length, unit: 'hours' },
    time_to_restore: { p50: percentile(restoresFromDetect, 0.5), p85: percentile(restoresFromDetect, 0.85), n: restoresFromDetect.length, unit: 'hours' },
  };
  const open_incidents = {
    n: openInc.length,
    oldest_age: openInc.length ? round2((w.end - Math.min(...openInc.map((x) => x.ts))) / DAY) : null,
    unit: 'days',
    deploys: openInc.map((x) => x.tags[0]),
  };

  // Hotfix rate, batch size, time to first commit.
  const hotfix_rate = { value: total ? round2(byKind.hotfix / total) : null, hotfix: byKind.hotfix, total };
  const shippedCount = new Map();
  for (const r of records) {
    if (!leadEligible(r)) continue;
    const e = recEvent.get(r);
    if (e && windowIds.has(e.id)) shippedCount.set(e.id, (shippedCount.get(e.id) || 0) + 1);
  }
  const batches = windowDeploys.map((d) => shippedCount.get(d.id) || 0).filter((n) => n > 0);
  const batch_size = { p50: percentile(batches, 0.5), p85: percentile(batches, 0.85), n: batches.length, unit: 'tickets', deploys_without_tickets: total - batches.length };
  const ttfc = [];
  for (const r of records) {
    const st = toMs(r.first_active_at ?? r.started_at);
    const fc = toMs(r.first_commit_at);
    if (!leadEligible(r) || !inWindow(toMs(r.deployed_at), w) || !Number.isFinite(st) || !Number.isFinite(fc) || fc < st) continue;
    ttfc.push(bdays(st, fc));
  }
  const time_to_first_commit = { p50: percentile(ttfc, 0.5), p85: percentile(ttfc, 0.85), n: ttfc.length, unit: 'business_days' };
  if (total > 0) weak('dora_mttr', restores.length, 'restored incidents');

  // Bugs created in the window that point at nothing: CFR can't see them.
  const unlinked = records.filter((r) => isBug(r) && !isSubtask(r) && inWindow(toMs(r.created ?? r.created_at), w) && !refsOf(r).length).map((r) => r.key);
  if (unlinked.length) {
    guardrails.push({ id: 'unlinked_bugs_in_window', metric: 'dora_cfr', code: 'unlinked_bugs', severity: 'weak', n: unlinked.length, keys: unlinked.slice(0, 50), reason: `${unlinked.length} bug(s) in the period are not linked to the ticket that caused them — the change failure rate may be understated.` });
  }

  return {
    deploy_frequency, lead_time, ticket_lead_time, change_failure_rate, mttr, open_incidents, hotfix_rate, batch_size, time_to_first_commit,
    deploys_per_capacity_week: deploy_frequency.per_capacity_week ?? null,
    guardrails, bands: DORA_BANDS, window: w, timezone: zone, weekend, failure_window_days: failWinMs / DAY, min_n: minN,
  };
}

module.exports = { doraMetrics, DORA_BANDS, bandFor, percentile, deployKind, hotfixPatterns, lowerBound, DEFAULT_HOTFIX_PATTERNS };
