// Guardrails: tell the reader WHEN a number must not be read at face value.
// evaluate(metrics, inputs) → [{ code, metric, severity, reason, action }]
//   metrics = { <metricKey>: { value, n, coverage, weak_reasons[] } } (lib/flow.js envelopes)
//   inputs  = scope-wide facts about the data feeding the metrics:
//     pr_count, delivered_count, design_tracked_share (0..1), estimate_coverage (0..1),
//     git_evidence_share (0..1), target_ref: { name, stale, behind_days },
//     unmapped_authors: [name], capacity: { capacity_days, business_days }, deploy_tags
// metric = the affected metric key, or '*' for scope-wide warnings.
'use strict';

const MIN_N = 5;
const SEV = { warning: 1, danger: 2 };

// Metric keys each scope-wide input weakens (used for badges).
const AFFECTS = {
  no_pr_data: ['cycleTimeByPhase', 'prPickup', 'prReview', 'prSize', 'leadTime'],
  design_not_tracked_share: ['cycleTimeByPhase'],
  low_estimate_coverage: ['estimateAccuracy', 'throughputPerWeek'],
  no_git_evidence: ['investmentMix', 'contextSwitching', 'cycleTimeByPhase', 'estimateAccuracy'],
  stale_target_ref: ['deploymentFrequency', 'leadTime', 'changeFailureRate', 'rework'],
  unmapped_authors: ['contextSwitching', 'throughputPerWeek', 'rework'],
  low_capacity: ['throughputPerWeek'],
  no_deploy_tags: ['deploymentFrequency', 'leadTime', 'changeFailureRate', 'mttr'],
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

module.exports = { MIN_N, AFFECTS, TILE_OF, evaluate, badgeFor, tileOf, canonicalize, envelopesOf, lowNGuardrails, uncovered };
