// Guardrails: tell the reader WHEN a number must not be read at face value.
// evaluate(metrics, inputs) → [{ code, metric, severity, reason, action }]
//   metrics = { <metricKey>: { value, n, coverage, weak_reasons[] } } (lib/flow.js envelopes)
//   inputs  = scope-wide facts about the data feeding the metrics:
//     pr_count, delivered_count, design_tracked_share (0..1), estimate_coverage (0..1),
//     git_evidence_share (0..1), target_ref: { name, stale, behind_days },
//     unmapped_authors: [name], capacity: { capacity_days, business_days }, deploy_tags,
//     pr_cache: { in_progress, age_days, partial_count }, jira_rework_low_confidence_share (0..1),
//     deploy_kinds: { regular, hotfix }
// metric = the affected metric key, or '*' for scope-wide warnings.
'use strict';

const MIN_N = 5;
const SEV = { warning: 1, danger: 2 };

// Metric keys each scope-wide input weakens (used for badges).
const AFFECTS = {
  no_pr_data: ['cycleTimeByPhase', 'prPickup', 'prReview', 'prSize', 'prDepth', 'prMerge', 'reviewLoad', 'leadTime'],
  design_not_tracked_share: ['cycleTimeByPhase'],
  low_estimate_coverage: ['estimateAccuracy', 'throughputPerWeek', 'estimateCalibration', 'estimateInflation'],
  no_git_evidence: ['investmentMix', 'contextSwitching', 'focus', 'flowEfficiency', 'cycleTimeByPhase', 'estimateAccuracy', 'reworkRateAll', 'reworkRateStrong'],
  stale_target_ref: ['deploymentFrequency', 'leadTime', 'changeFailureRate', 'mttr', 'deploysPerCapacityWeek', 'rework', 'reworkRateAll', 'reworkRateStrong', 'escapeRate'],
  unmapped_authors: ['contextSwitching', 'focus', 'throughputPerWeek', 'rework', 'reworkRateAll', 'reworkRateStrong', 'reviewLoad'],
  low_capacity: ['throughputPerWeek', 'deploysPerCapacityWeek', 'reviewLoad'],
  no_deploy_tags: ['deploymentFrequency', 'leadTime', 'changeFailureRate', 'mttr', 'deploysPerCapacityWeek'],
  // PR cache still syncing / stale / holding partial PRs → every PR-derived tile.
  pr_cache_partial: ['prPickup', 'prReview', 'prSize', 'prDepth', 'prMerge', 'reviewLoad', 'cycleTimeByPhase'],
  // Most Jira-detected rework links are low-confidence heuristics.
  jira_rework_low_confidence: ['reworkRateAll', 'reworkRateStrong', 'fixRate', 'changeFailureRate', 'escapeRate'],
  hf_only_tags: ['deploymentFrequency', 'deploysPerCapacityWeek', 'changeFailureRate'],
};

const pct = (x) => `${Math.round(x * 100)}%`;

function evaluate(metrics = {}, inputs = {}) {
  const out = [];
  const add = (code, metric, severity, reason, action) => out.push({ code, metric, severity, reason, action });
  const spread = (code, severity, reason, action) => add(code, '*', severity, reason, action);

  for (const [key, m] of Object.entries(metrics)) {
    if (!m || typeof m !== 'object' || m.n == null) continue;
    if (m.n < MIN_N) {
      add('low_n', key, m.n < 2 ? 'danger' : 'warning',
        `Only ${m.n} item${m.n === 1 ? '' : 's'} behind this number — one outlier moves it a lot.`,
        'Widen the period or the team scope before comparing.');
    }
  }

  const delivered = inputs.delivered_count || 0;
  if (inputs.pr_count != null && delivered > 0 && inputs.pr_count === 0) {
    spread('no_pr_data', 'danger', 'No pull requests were fetched for this scope, so review and pickup times are unknown.',
      'Run the PR sync (Bitbucket via Otto) and check the repo↔account binding.');
  }
  if (inputs.design_tracked_share != null && delivered > 0 && inputs.design_tracked_share < 0.5) {
    spread('design_not_tracked_share', 'warning',
      `Design time is tracked on only ${pct(inputs.design_tracked_share)} of delivered tickets — the design phase is "not tracked", not zero.`,
      'Log design/spike sub-tasks or use a design status to capture it.');
  }
  if (inputs.estimate_coverage != null && delivered > 0 && inputs.estimate_coverage < 0.7) {
    add('low_estimate_coverage', 'estimateAccuracy', inputs.estimate_coverage < 0.4 ? 'danger' : 'warning',
      `Only ${pct(inputs.estimate_coverage)} of delivered tickets have an estimate.`,
      'Run estimation for the missing tickets before reading accuracy or weighted throughput.');
  }
  if (inputs.git_evidence_share != null && delivered > 0 && inputs.git_evidence_share < 0.7) {
    spread('no_git_evidence', inputs.git_evidence_share < 0.5 ? 'danger' : 'warning',
      `${pct(1 - inputs.git_evidence_share)} of delivered tickets have no keyed commits — their dev time comes from Jira statuses alone.`,
      'Put the ticket key in commit subjects / branch names; check the repo list.');
  }
  if (inputs.target_ref && inputs.target_ref.stale) {
    const r = inputs.target_ref;
    spread('stale_target_ref', 'warning',
      `Main branch ${r.name || ''} looks stale${r.behind_days != null ? ` (${r.behind_days} days behind)` : ''} — merges and deploys after it are missed.`,
      'Fetch the repo, prefer origin/develop over a local branch.');
  }
  const unmapped = inputs.unmapped_authors || [];
  if (unmapped.length) {
    spread('unmapped_authors', unmapped.length > 3 ? 'danger' : 'warning',
      `${unmapped.length} git author${unmapped.length === 1 ? ' is' : 's are'} not mapped to a person — their commits are not attributed.`,
      'Map the aliases in the People editor.');
  }
  const cap = inputs.capacity;
  if (cap && cap.business_days > 0 && cap.capacity_days / cap.business_days < 0.5) {
    add('low_capacity', 'throughputPerWeek', cap.capacity_days === 0 ? 'danger' : 'warning',
      `Available on ${cap.capacity_days} of ${cap.business_days} working days (time off) — rates are per available day, totals are naturally lower.`,
      'Compare per-capacity-day rates, not raw totals.');
  }
  if (inputs.deploy_tags != null && inputs.deploy_tags === 0) {
    spread('no_deploy_tags', 'danger', 'No deployment tags (name containing "deployed", "hf" or "hotfix") found in this window — DORA metrics are unavailable.',
      'Tag deployments, or check the tag fetch for the repos.');
  }
  const pc = inputs.pr_cache;
  if (pc && (pc.in_progress || pc.age_days > 2 || pc.partial_count > 0)) {
    const bits = [pc.in_progress ? 'a sync is still running' : null, pc.age_days > 2 ? `it is ${Math.round(pc.age_days)} days old` : null,
      pc.partial_count > 0 ? `${pc.partial_count} PR(s) are only partially fetched` : null].filter(Boolean);
    spread('pr_cache_partial', 'warning', `The PR cache is incomplete: ${bits.join(', ')}.`, 'Let the PR sync finish (or re-run it) before reading PR timings.');
  }
  const lc = inputs.jira_rework_low_confidence_share;
  if (lc != null && lc > 0.5) {
    spread('jira_rework_low_confidence', 'warning', `${pct(lc)} of Jira-detected rework links are low-confidence heuristics (title/“follow-up” matches).`,
      'Confirm links in Jira (caused-by / fixes) before reading rework or failure rates.');
  }
  const dk = inputs.deploy_kinds;
  if (dk && (dk.regular || 0) === 0 && (dk.hotfix || 0) > 0) {
    spread('hf_only_tags', 'warning', 'Every deployment tag in this window is a hotfix/hf tag — regular releases are not tagged, so frequency and failure rate are skewed.',
      'Tag regular releases with "deployed".');
  }
  return out.sort((a, b) => SEV[b.severity] - SEV[a.severity] || a.code.localeCompare(b.code));
}

/** Worst guardrail touching a metric: { severity, codes, reasons } or null. */
function badgeFor(metricKey, list) {
  const hits = (list || []).filter((g) => g.metric === metricKey || (g.metric === '*' && (AFFECTS[g.code] || []).includes(metricKey)));
  if (!hits.length) return null;
  const severity = hits.some((g) => g.severity === 'danger') ? 'danger' : 'warning';
  return { severity, codes: [...new Set(hits.map((g) => g.code))], reasons: hits.map((g) => g.reason) };
}

// ---- canonical list (ONE array for the overview + reports) ------------------
// Every guardrail gets a stable `id` that is a UI tile key, so a tile can find
// its own warning with list.filter(g => g.id === tile || g.tiles.includes(tile)).

// metric key (flow envelope / computeMetrics) → tile key.
const TILE_OF = {
  deploymentFrequency: 'dora_deploy_frequency', leadTime: 'dora_lead_time', changeFailureRate: 'dora_cfr', mttr: 'dora_mttr',
  'dora.deploy_frequency': 'dora_deploy_frequency', 'dora.lead_time': 'dora_lead_time', 'dora.change_failure_rate': 'dora_cfr', 'dora.mttr': 'dora_mttr',
  prPickup: 'pr_pickup', prReview: 'pr_review', prSize: 'pr_size', prMerge: 'pr_merge', prUnreviewed: 'pr_unreviewed',
  estimateAccuracy: 'estimate_accuracy',
  throughputPerWeek: 'capacity_throughput', wip: 'capacity_wip', agingWip: 'capacity_aging_wip', contextSwitching: 'capacity_context_switching',
  investmentMix: 'capacity_investment', unplannedShare: 'capacity_unplanned', rework: 'capacity_rework',
  cycleTimeByPhase: 'phase_cycle_weak',
  prDepth: 'pr_depth', reviewLoad: 'pr_review_load', mergesPerCapacityWeek: 'pr_merges_per_capacity_week',
  reworkRateAll: 'capacity_rework_rate_all', reworkRateStrong: 'capacity_rework_rate_strong', fixRate: 'capacity_fix_rate',
  flowEfficiency: 'capacity_flow_efficiency', focus: 'capacity_focus', focusShare: 'capacity_focus',
  escapeRate: 'capacity_escape', sprintPlanning: 'capacity_sprint_planning',
  deploysPerCapacityWeek: 'dora_deploys_per_capacity_week',
  estimateCalibration: 'estimate_calibration', estimateInflation: 'estimate_inflation',
};
const phaseTile = (name) => `phase_${String(name).replace(/[^a-z0-9]+/gi, '_').toLowerCase()}_weak`;
function tileOf(metric) {
  metric = String(metric).replace(/^dora\.(?=dora_)/, ''); // lib/dora ids already carry the prefix
  if (TILE_OF[metric]) return TILE_OF[metric];
  const m = /^phases?\.(.+)$/.exec(String(metric));
  if (m) return phaseTile(m[1]);
  return String(metric).replace(/[^a-z0-9]+/gi, '_').toLowerCase();
}

// scope-wide code → its own stable id (category-prefixed).
const SCOPE_ID = {
  no_pr_data: 'pr_no_data', pr_times_approximated: 'pr_times_approximated',
  design_not_tracked_share: 'phase_design_weak', no_git_evidence: 'phase_dev_weak',
  low_estimate_coverage: 'estimate_coverage_low',
  stale_target_ref: 'dora_stale_target_ref', no_deploy_tags: 'dora_no_deploy_tags',
  unmapped_authors: 'capacity_unmapped_authors', low_capacity: 'capacity_low',
  pr_cache_partial: 'pr_cache_partial', jira_rework_low_confidence: 'capacity_rework_low_confidence', hf_only_tags: 'dora_hf_only_tags',
};

/**
 * Raw guardrails (evaluate() + extras) → ONE canonical, de-duplicated array:
 * [{ id, code, metric, tiles[], severity, level:'warn'|'bad', reason, action, msg }].
 * Same id twice keeps the worse severity and joins the reasons.
 */
const TILE_ID_RE = /^(dora|phase|pr|estimate|capacity)_[a-z0-9_]+$/;
// lib severities → the two levels the UI renders ('weak' is a warning; a
// metric that is 'not_available' is as bad as it gets).
const normSev = (s) => (s === 'danger' || s === 'not_available' || s === 'error' || s === 'bad' ? 'danger' : 'warning');

function canonicalize(list) {
  const byId = new Map();
  for (const g0 of list || []) {
    if (!g0 || !g0.code) continue;
    const g = { ...g0, severity: normSev(g0.severity) };
    const scopeWide = g.metric === '*' || g.metric == null;
    const tile = scopeWide ? null : tileOf(g.metric);
    const id = SCOPE_ID[g.code]
      || (g.code === 'low_n' || g.code === 'not_available' || g.code === 'empty_window' ? tile : null)
      || (TILE_ID_RE.test(g.code) ? g.code : null)
      || (scopeWide ? g.code : `${tile}_${g.code}`);
    const tiles = scopeWide ? (AFFECTS[g.code] || []).map(tileOf) : [tileOf(g.metric)];
    if (SCOPE_ID[g.code] === 'phase_design_weak') tiles.push('phase_design_weak');
    if (SCOPE_ID[g.code] === 'phase_dev_weak') tiles.push('phase_dev_weak');
    const prev = byId.get(id);
    if (prev) {
      if (SEV[g.severity] > SEV[prev.severity]) prev.severity = g.severity;
      if (!prev.reason.includes(g.reason)) prev.reason = `${prev.reason} ${g.reason}`;
      for (const t of tiles) if (!prev.tiles.includes(t)) prev.tiles.push(t);
      continue;
    }
    byId.set(id, { id, code: g.code, metric: g.metric ?? '*', tiles: [...new Set(tiles)], severity: g.severity, reason: String(g.reason || ''), action: String(g.action || '') });
  }
  return [...byId.values()]
    .map((g) => ({ ...g, level: g.severity === 'danger' ? 'bad' : 'warn', msg: `${g.reason}${g.action ? ` ${g.action}` : ''}` }))
    .sort((a, b) => SEV[b.severity] - SEV[a.severity] || a.id.localeCompare(b.id));
}

const nOf = (m) => {
  if (!m || typeof m !== 'object') return null;
  for (const k of ['n', 'total', 'deployments']) if (Number.isFinite(m[k])) return m[k];
  return null;
};

/**
 * Flatten a computeMetrics() result into { metricKey: { n } } for every metric
 * that carries a sample size — flow envelopes, phases.*, DORA, PR flow.
 */
function envelopesOf(met = {}) {
  const out = {};
  for (const [k, m] of Object.entries(met.flow || {})) if (nOf(m) != null) out[k] = { n: nOf(m) };
  for (const [k, m] of Object.entries(met.phases || {})) if (nOf(m) != null) out[`phases.${k}`] = { n: nOf(m) };
  const d = met.dora || {};
  if (d.deploy_frequency && nOf(d.deploy_frequency) != null) out.deploymentFrequency = { n: nOf(d.deploy_frequency) };
  if (d.lead_time && nOf(d.lead_time) != null) out.leadTime = { n: nOf(d.lead_time) };
  if (d.change_failure_rate) { const n = nOf(d.change_failure_rate) ?? d.change_failure_rate.deployments; if (n != null) out.changeFailureRate = { n }; }
  if (d.mttr && nOf(d.mttr) != null) out.mttr = { n: nOf(d.mttr) };
  const pf = met.pr_flow || {};
  const prMap = { pickup_days: 'prPickup', review_days: 'prReview', merge_lag_days: 'prMerge', size: 'prSize', unreviewed: 'prUnreviewed' };
  for (const [k, key] of Object.entries(prMap)) if (pf[k] && nOf(pf[k]) != null) out[key] = { n: nOf(pf[k]) };
  return out;
}

/** low_n guardrails for every envelope; a metric with n === 0 is "no data", still flagged. */
function lowNGuardrails(envs, minN = MIN_N) {
  return Object.entries(envs).filter(([, m]) => m.n < minN).map(([key, m]) => ({
    code: 'low_n', metric: key, severity: m.n < 2 ? 'danger' : 'warning',
    reason: m.n === 0 ? 'No data behind this number in the window.' : `Only ${m.n} item${m.n === 1 ? '' : 's'} behind this number — one outlier moves it a lot.`,
    action: 'Widen the period or the team scope before comparing.',
  }));
}

/** Metric keys with n < minN that have NO matching canonical guardrail (should be []). */
function uncovered(envs, canonList, minN = MIN_N) {
  return Object.entries(envs).filter(([k, m]) => m.n < minN && !canonList.some((g) => g.id === tileOf(k) || g.tiles.includes(tileOf(k)))).map(([k]) => k);
}

// ---- per-tile guards (guardFor) + the coverage contract ---------------------
// guardFor(met, keys) reads a computeMetrics() result directly and answers
// "can this tile be read at face value?" with short inline reasons
// ('n=3, too few deploys'). COVERAGE maps every numeric leaf path of that
// result onto the tile whose guard protects it; WHITELIST lists the leaves
// that are inputs/denominators/config, not readable metrics. The contract test
// fails on any numeric leaf in neither — a new metric must bring its guard.

const DAY_MS = 86400000;
const PR_STALE_DAYS = 2;
const num = (x) => (Number.isFinite(x) ? x : null);
const get = (o, path) => path.split('.').reduce((a, k) => (a == null ? undefined : a[k]), o);
const firstOf = (o, paths) => { for (const p of paths) { const v = get(o, p); if (v != null) return v; } return undefined; };
const share = (part, whole) => (whole > 0 && Number.isFinite(part) ? part / whole : null);

/** Sample size of a metric node (envelope n, DORA total, PR n …). */
const sizeOf = (node) => (node == null ? null : typeof node === 'number' ? null : nOf(node));

/** PR-cache health → reasons (cursor.in_progress, synced_at age, partial PRs). */
function prCacheReasons(met, now) {
  const st = met.pr_sync || met.pr_status || (met.pr_flow && met.pr_flow.sync) || null;
  if (!st) return [];
  const out = [];
  const repos = Object.values(st.cursor_by_repo || {});
  if (st.cursor && !repos.length) repos.push(st.cursor);
  const syncing = repos.filter((c) => c && c.in_progress).length + (st.in_progress ? 1 : 0);
  if (syncing) out.push(`PR sync in progress (${syncing} repo${syncing === 1 ? '' : 's'})`);
  if (st.state && st.state !== 'ok') out.push(`PR sync ${String(st.state).replace(/_/g, ' ')}`);
  const synced = [st.synced_at, ...repos.map((c) => c && c.synced_at)].map((t) => (t == null ? null : typeof t === 'number' ? t : Date.parse(t))).filter(Number.isFinite);
  if (synced.length && now != null) {
    const age = (now - Math.min(...synced)) / DAY_MS;
    if (age > PR_STALE_DAYS) out.push(`PR cache ${Math.round(age)}d old`);
  }
  const partial = num(firstOf(met, ['pr_sync.partial_count', 'pr_sync.partial', 'pr_flow.partial_count', 'pr_flow.partial.count', 'pr_flow.partial']));
  if (partial > 0) out.push(`${partial} PR${partial === 1 ? '' : 's'} partially fetched`);
  return out;
}

const lowN = (noun, minN = MIN_N) => (met, node) => {
  const n = sizeOf(node);
  if (n == null) return ['no data'];
  return n < minN ? [`n=${n}, too few ${noun}`] : [];
};
const lowCoverage = (node, what = 'tickets') => {
  const c = node && num(node.coverage);
  return c != null && c < 0.5 ? [`coverage ${pct(c)} of ${what}`] : [];
};
const weakReasons = (node) => (node && Array.isArray(node.weak_reasons) ? node.weak_reasons.filter((r) => r !== 'low_n').map((r) => String(r).replace(/_/g, ' ')) : []);

const deployNode = (met) => get(met, 'dora.deploy_frequency');
const hfOnly = (met) => {
  const k = get(met, 'dora.deploy_frequency.by_kind');
  return k && (k.regular || 0) === 0 && (k.hotfix || 0) > 0 ? ['only hotfix/hf tags matched'] : [];
};
const jiraLowConf = (met) => {
  const s = num(get(met, 'rework.jira_low_confidence_share'));
  if (s != null) return s > 0.5 ? [`${pct(s)} of Jira rework links low-confidence`] : [];
  const items = get(met, 'rework.items');
  if (!Array.isArray(items) || !items.length) return [];
  const low = items.filter((i) => i && i.confidence === 'low').length / items.length;
  return low > 0.5 ? [`${pct(low)} of Jira rework links low-confidence`] : [];
};
const reworkN = (met) => num(firstOf(met, ['rework.n', 'rework.delivered', 'rework.total']));
const reworkGuard = (met) => {
  const n = reworkN(met);
  return [...(n == null ? ['no data'] : n < MIN_N ? [`n=${n}, too few delivered tickets`] : []), ...jiraLowConf(met)];
};
const capacityWeeks = (met) => {
  const c = num(get(met, 'capacity.capacity_days'));
  return c == null ? null : c / 5;
};

/**
 * Tile → { node(met) → metric node, check(met, node, ctx) → reasons[] }.
 * Tile keys are the canonical ids from tileOf(), so a tile asks with its own id.
 */
const TILE_GUARDS = {
  dora_deploy_frequency: { node: deployNode, check: (m, n) => [...lowN('deploys')(m, n), ...hfOnly(m)] },
  dora_lead_time: { node: (m) => get(m, 'dora.lead_time'), check: lowN('changes') },
  dora_ticket_lead_time: { node: (m) => get(m, 'dora.ticket_lead_time'), check: lowN('tickets') },
  dora_cfr: { node: (m) => get(m, 'dora.change_failure_rate'), check: (m, n) => [...lowN('deploys')(m, n), ...hfOnly(m), ...jiraLowConf(m)] },
  dora_open_incidents: { node: (m) => get(m, 'dora.open_incidents'), check: lowN('incidents') },
  dora_hotfix_rate: { node: (m) => { const h = get(m, 'dora.hotfix_rate'); return h && { ...h, n: h.n ?? h.total }; }, check: (m, n) => [...lowN('deploys')(m, n), ...hfOnly(m)] },
  dora_batch_size: { node: (m) => get(m, 'dora.batch_size'), check: lowN('deploys') },
  dora_time_to_first_commit: { node: (m) => get(m, 'dora.time_to_first_commit'), check: lowN('tickets') },
  dora_mttr: {
    node: (m) => get(m, 'dora.mttr'),
    check: (m, n) => {
      const out = lowN('incidents')(m, n);
      const s = share(num(n && n.from_deploy), sizeOf(n));
      if (s != null && s > 0.5) out.push(`${pct(s)} of restores timed from the deploy, not the incident`);
      return out;
    },
  },
  dora_deploys_per_capacity_week: {
    node: (m) => firstOf(m, ['flow.deploysPerCapacityWeek', 'dora.deploys_per_capacity_week']) || deployNode(m),
    check: (m) => {
      const out = lowN('deploys')(m, deployNode(m));
      const w = capacityWeeks(m);
      if (w != null && w < 1) out.push(`only ${Math.round(w * 5)} capacity days`);
      return [...out, ...hfOnly(m)];
    },
  },
  pr_pickup: { node: (m) => get(m, 'pr_flow.pickup_days'), check: (m, n, c) => [...lowN('PRs')(m, n), ...c.pr] },
  pr_review: { node: (m) => get(m, 'pr_flow.review_days'), check: (m, n, c) => [...lowN('PRs')(m, n), ...c.pr] },
  pr_merge: { node: (m) => get(m, 'pr_flow.merge_lag_days'), check: (m, n, c) => [...lowN('PRs')(m, n), ...c.pr] },
  pr_cycle: { node: (m) => get(m, 'pr_flow.pr_cycle_days'), check: (m, n, c) => [...lowN('PRs')(m, n), ...c.pr] },
  pr_size: { node: (m) => get(m, 'pr_flow.size'), check: (m, n, c) => [...lowN('PRs')(m, n), ...c.pr] },
  pr_depth: { node: (m) => get(m, 'pr_flow.review_depth'), check: (m, n, c) => [...lowN('reviewed PRs')(m, n), ...c.pr] },
  pr_unreviewed: { node: (m) => get(m, 'pr_flow.unreviewed'), check: (m, n, c) => [...lowN('PRs')(m, n), ...c.pr] },
  pr_merges_per_capacity_week: {
    node: (m) => ({ n: num(get(m, 'pr_flow.total')) ?? num(get(m, 'pr_flow.counts.merged')) }),
    check: (m, n, c) => {
      const out = [...lowN('PRs')(m, n), ...c.pr];
      const w = capacityWeeks(m);
      if (w != null && w < 1) out.push(`only ${Math.round(w * 5)} capacity days`);
      return out;
    },
  },
  pr_review_load: { node: (m) => firstOf(m, ['pr_flow.review_load', 'review_load']), check: (m, n, c) => [...lowN('reviews')(m, n), ...c.pr] },
  pr_counts: { node: (m) => ({ n: num(get(m, 'pr_flow.total')) ?? num(get(m, 'pr_flow.counts.total')) }), check: (m, n, c) => [...lowN('PRs')(m, n), ...c.pr] },
  capacity_throughput: {
    node: (m) => get(m, 'flow.throughputPerWeek'),
    check: (m, n) => {
      const out = [...lowN('delivered tickets')(m, n), ...lowCoverage(n)];
      const cap = get(m, 'capacity');
      if (cap && cap.business_days > 0 && cap.capacity_days / cap.business_days < 0.5) out.push(`capacity ${cap.capacity_days}/${cap.business_days} days`);
      return out;
    },
  },
  capacity_wip: { node: (m) => get(m, 'flow.wip'), check: lowN('open tickets') },
  capacity_aging_wip: { node: (m) => get(m, 'flow.agingWip'), check: lowN('open tickets') },
  capacity_context_switching: { node: (m) => get(m, 'flow.contextSwitching'), check: (m, n) => [...lowN('active days')(m, n), ...lowCoverage(n, 'days')] },
  capacity_focus: { node: (m) => firstOf(m, ['flow.focus', 'flow.focusShare']), check: (m, n) => [...lowN('active days')(m, n), ...lowCoverage(n, 'days')] },
  capacity_flow_efficiency: { node: (m) => get(m, 'flow.flowEfficiency'), check: (m, n) => [...lowN('done tickets')(m, n), ...lowCoverage(n), ...weakReasons(n)] },
  capacity_investment: { node: (m) => get(m, 'flow.investmentMix'), check: (m, n) => [...lowN('tickets with dev time')(m, n), ...lowCoverage(n), ...weakReasons(n)] },
  capacity_investment_epic: { node: (m) => get(m, 'flow.investmentByEpic'), check: (m, n) => [...lowN('tickets with dev time')(m, n), ...weakReasons(n)] },
  capacity_unplanned: { node: (m) => get(m, 'flow.unplannedShare'), check: (m, n) => [...lowN('delivered tickets')(m, n), ...lowCoverage(n)] },
  capacity_escape: { node: (m) => get(m, 'flow.escapeRate'), check: (m, n) => [...lowN('delivered tickets')(m, n), ...jiraLowConf(m)] },
  capacity_sprint_planning: { node: (m) => get(m, 'flow.sprintPlanning'), check: (m, n) => [...lowN('planned tickets')(m, n), ...weakReasons(n)] },
  capacity_rework: { node: (m) => get(m, 'rework'), check: reworkGuard },
  capacity_rework_rate_all: { node: (m) => get(m, 'rework'), check: reworkGuard },
  capacity_rework_rate_strong: { node: (m) => get(m, 'rework'), check: reworkGuard },
  capacity_fix_rate: { node: (m) => get(m, 'rework'), check: reworkGuard },
  capacity_rework_by_person: { node: (m) => get(m, 'flow.reworkRateByPerson'), check: lowN('delivered tickets') },
  phase_cycle_weak: { node: (m) => get(m, 'flow.cycleTimeByPhase'), check: (m, n) => [...lowN('tickets')(m, n), ...lowCoverage(n)] },
  phase_weak: {
    node: (m) => get(m, 'phases'),
    check: (m, ph) => Object.entries(ph || {}).flatMap(([k, s]) => (s && typeof s === 'object' && s.n != null && (s.n < MIN_N || (s.coverage != null && s.coverage < 0.5)) ? [`${k.replace(/_/g, ' ')}: n=${s.n}`] : [])),
  },
  estimate_accuracy: { node: (m) => get(m, 'flow.estimateAccuracy'), check: (m, n) => [...lowN('estimated tickets')(m, n), ...lowCoverage(n)] },
  estimate_calibration: { node: (m) => firstOf(m, ['estimates.calibration', 'estimate_quality.calibration']), check: lowN('lead corrections') },
  estimate_inflation: {
    node: (m) => firstOf(m, ['estimates.inflation', 'estimate_quality.inflation']),
    check: (m, n) => {
      if (!n) return ['no data'];
      const a = num(n.n); const b = num(n.n_ref);
      const out = [];
      if (a == null || a < MIN_N) out.push(`n=${a ?? 0}, too few re-estimated tickets`);
      if (b != null && b < MIN_N) out.push(`n_ref=${b}, thin reference period`);
      return out;
    },
  },
};

/** Resolve a key (tile id, metric key, or phases.<x>) onto a TILE_GUARDS id. */
function guardKey(k) {
  if (TILE_GUARDS[k]) return k;
  const t = tileOf(k);
  if (TILE_GUARDS[t]) return t;
  if (/^phase_.+_weak$/.test(t)) return 'phase_weak';
  return null;
}

/**
 * Worst guard over `keys` (tile ids or metric keys) for a computeMetrics() result.
 * → { level: 'ok'|'warn'|'bad', reasons: string[] }. 'bad' = no data at all or n < 2.
 * opts.now (ms) ages the PR cache; defaults to window.until, then Date.now().
 */
function guardFor(met = {}, keys = [], opts = {}) {
  const now = opts.now ?? get(met, 'window.until') ?? Date.now();
  const ctx = { pr: prCacheReasons(met, now) };
  const reasons = [];
  let bad = false;
  for (const k of [].concat(keys)) {
    const gk = guardKey(k);
    if (!gk) continue;
    const g = TILE_GUARDS[gk];
    let node;
    try { node = g.node(met); } catch { node = undefined; }
    const rs = g.check(met, node, ctx) || [];
    for (const r of rs) {
      if (r === 'no data' || /^n=[01],/.test(r)) bad = true;
      if (!reasons.includes(r)) reasons.push(r);
    }
  }
  return { level: bad ? 'bad' : reasons.length ? 'warn' : 'ok', reasons };
}

// Numeric leaf path (arrays as `[]`) → tile whose guard covers it. First match wins.
const COVERAGE = [
  [/^dora\.deploy_frequency\./, 'dora_deploy_frequency'],
  [/^dora\.lead_time\./, 'dora_lead_time'],
  [/^dora\.ticket_lead_time\./, 'dora_ticket_lead_time'],
  [/^dora\.change_failure_rate\./, 'dora_cfr'],
  [/^dora\.mttr\./, 'dora_mttr'],
  [/^dora\.open_incidents\./, 'dora_open_incidents'],
  [/^dora\.hotfix_rate\./, 'dora_hotfix_rate'],
  [/^dora\.batch_size\./, 'dora_batch_size'],
  [/^dora\.time_to_first_commit\./, 'dora_time_to_first_commit'],
  [/^(dora\.deploys_per_capacity_week|flow\.deploysPerCapacityWeek)(\.|$)/, 'dora_deploys_per_capacity_week'],
  [/^pr_flow\.pickup_days\./, 'pr_pickup'],
  [/^pr_flow\.(review_days|review_rounds|post_review_commits|reworked_after_review|merged_without_approval)\./, 'pr_review'],
  [/^pr_flow\.(merge_lag_days|coding_days)\./, 'pr_merge'],
  [/^pr_flow\.pr_cycle_days\./, 'pr_cycle'],
  [/^pr_flow\.(size|size_buckets)\./, 'pr_size'],
  [/^pr_flow\.(review_depth|comments_count|reviewers_n)\./, 'pr_depth'],
  [/^pr_flow\.(unreviewed\.|unreviewed_share$)/, 'pr_unreviewed'],
  [/^pr_flow\.merges_per_capacity_week(\.|$)/, 'pr_merges_per_capacity_week'],
  [/^(pr_flow\.)?review_load(\.|$)/, 'pr_review_load'],
  [/^pr_flow\.(counts\.|total$|approximated_times$|partial_count$|partial$|partial\.)/, 'pr_counts'],
  [/^flow\.throughputPerWeek\./, 'capacity_throughput'],
  [/^flow\.wip\./, 'capacity_wip'],
  [/^flow\.agingWip\./, 'capacity_aging_wip'],
  [/^flow\.contextSwitching\./, 'capacity_context_switching'],
  [/^flow\.(focus|focusShare)\./, 'capacity_focus'],
  [/^flow\.flowEfficiency\./, 'capacity_flow_efficiency'],
  [/^(flow\.investmentMix\.|investment\.)/, 'capacity_investment'],
  [/^flow\.investmentByEpic\./, 'capacity_investment_epic'],
  [/^flow\.unplannedShare\./, 'capacity_unplanned'],
  [/^flow\.escapeRate\./, 'capacity_escape'],
  [/^flow\.sprintPlanning\./, 'capacity_sprint_planning'],
  [/^flow\.reworkRateByPerson\./, 'capacity_rework_by_person'],
  [/^rework\.(rate_all|rework_rate_all)$/, 'capacity_rework_rate_all'],
  [/^rework\.(rate_strong|rework_rate_strong)$/, 'capacity_rework_rate_strong'],
  [/^rework\.fix_rate$/, 'capacity_fix_rate'],
  [/^rework\./, 'capacity_rework'],
  [/^flow\.cycleTimeByPhase\./, 'phase_cycle_weak'],
  [/^(phases\.|phase_population$)/, 'phase_weak'],
  [/^(flow\.estimateAccuracy\.|estimate_accuracy\.)/, 'estimate_accuracy'],
  [/^(estimates|estimate_quality)\.calibration\./, 'estimate_calibration'],
  [/^(estimates|estimate_quality)\.inflation\./, 'estimate_inflation'],
];

// Numeric leaves that are NOT metrics a reader interprets: window bounds,
// static DORA band thresholds, guardrail bookkeeping, capacity denominators
// (they ARE the context other tiles are judged against), PR sync bookkeeping.
const WHITELIST = [
  /^window\./,
  /^dora\.window\./,
  /^dora\.bands\./,
  /^dora\.guardrails\[\]\./,
  /^dora\.timezone/,
  /^dora\.(weekend\[\]|failure_window_days|min_n)$/, // config echoed back
  /^guardrails\[\]\./,
  /^badges\./,
  /^capacity\./,
  /^(pr_sync|pr_status)\./,
  /^subtasks\[\]\.dev_days$/, // attribution list; the per-ticket number is shown as-is with its key
];

/** Every numeric leaf path in `obj` (arrays collapsed to `[]`), de-duplicated. */
function numericLeaves(obj) {
  const out = new Set();
  (function walk(o, p) {
    if (typeof o === 'number') { out.add(p); return; }
    if (!o || typeof o !== 'object') return;
    if (Array.isArray(o)) { for (const x of o) walk(x, `${p}[]`); return; }
    for (const k of Object.keys(o)) walk(o[k], p ? `${p}.${k}` : k);
  })(obj, '');
  return [...out];
}

/** Tile covering a numeric leaf path, 'whitelist', or null (unguarded). */
function coverageOf(path) {
  for (const [re, tile] of COVERAGE) if (re.test(path)) return tile;
  return WHITELIST.some((re) => re.test(path)) ? 'whitelist' : null;
}

/** Numeric leaf paths of a computeMetrics() result that no guard covers (should be []). */
function unguardedPaths(met) {
  return numericLeaves(met).filter((p) => coverageOf(p) == null);
}

module.exports = { MIN_N, AFFECTS, TILE_OF, evaluate, badgeFor, tileOf, canonicalize, envelopesOf, lowNGuardrails, uncovered,
  TILE_GUARDS, COVERAGE, WHITELIST, guardFor, numericLeaves, coverageOf, unguardedPaths };
