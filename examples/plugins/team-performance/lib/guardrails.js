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

module.exports = { MIN_N, AFFECTS, evaluate, badgeFor };
