// Unit tests for lib/guardrails.js — one scope per code + a healthy scope.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const G = require('../lib/guardrails.js');

const HEALTHY_IN = {
  delivered_count: 40, pr_count: 35, design_tracked_share: 0.8, estimate_coverage: 0.95, git_evidence_share: 0.9,
  target_ref: { name: 'origin/develop', stale: false }, unmapped_authors: [], capacity: { capacity_days: 18, business_days: 20 }, deploy_tags: 6,
};
const HEALTHY_M = { throughputPerWeek: { value: 4, n: 40, coverage: 1, weak_reasons: [] } };
const codes = (l) => l.map((g) => g.code);

test('healthy scope → []', () => {
  assert.deepEqual(G.evaluate(HEALTHY_M, HEALTHY_IN), []);
});

const CASES = [
  ['low_n', { throughputPerWeek: { value: 1, n: 3, coverage: 1, weak_reasons: ['low_n'] } }, {}],
  ['no_pr_data', HEALTHY_M, { pr_count: 0 }],
  ['design_not_tracked_share', HEALTHY_M, { design_tracked_share: 0.2 }],
  ['low_estimate_coverage', HEALTHY_M, { estimate_coverage: 0.5 }],
  ['no_git_evidence', HEALTHY_M, { git_evidence_share: 0.4 }],
  ['stale_target_ref', HEALTHY_M, { target_ref: { name: 'develop', stale: true, behind_days: 30 } }],
  ['unmapped_authors', HEALTHY_M, { unmapped_authors: ['someone'] }],
  ['low_capacity', HEALTHY_M, { capacity: { capacity_days: 5, business_days: 20 } }],
  ['no_deploy_tags', HEALTHY_M, { deploy_tags: 0 }],
];
for (const [code, m, over] of CASES) {
  test(`guardrail ${code}`, () => {
    const list = G.evaluate(m, { ...HEALTHY_IN, ...over });
    assert.deepEqual(codes(list), [code]);
    const g = list[0];
    assert.ok(['warning', 'danger'].includes(g.severity));
    assert.ok(g.reason && g.action && g.metric);
  });
}

test('severity escalation and ordering', () => {
  const list = G.evaluate(HEALTHY_M, { ...HEALTHY_IN, estimate_coverage: 0.3, unmapped_authors: ['x'], deploy_tags: 0 });
  assert.deepEqual(list.map((g) => [g.code, g.severity]), [['low_estimate_coverage', 'danger'], ['no_deploy_tags', 'danger'], ['unmapped_authors', 'warning']]);
});

test('badgeFor: direct and scope-wide hits', () => {
  const list = G.evaluate({ estimateAccuracy: { n: 1 } }, { ...HEALTHY_IN, deploy_tags: 0 });
  assert.deepEqual(G.badgeFor('estimateAccuracy', list).codes, ['low_n']);
  assert.equal(G.badgeFor('leadTime', list).severity, 'danger');
  assert.equal(G.badgeFor('wip', list), null);
});

// ---- canonical list ---------------------------------------------------------

test('canonicalize: stable tile ids, dedupe keeps worst severity, scope-wide tiles', () => {
  const raw = [
    ...G.evaluate({ estimateAccuracy: { n: 3 } }, { ...HEALTHY_IN, deploy_tags: 0, pr_count: 0 }),
    { code: 'low_n', metric: 'dora.dora_lead_time', severity: 'weak', reason: 'x' },
    { code: 'phase_dev_weak', metric: 'phases.dev', severity: 'warning', reason: 'dev weak' },
    { code: 'low_n', metric: 'phases.dev', severity: 'danger', reason: 'n=1' },
    { code: 'not_available', metric: 'dora.dora_deploy_frequency', severity: 'not_available', reason: 'no tags' },
  ];
  const list = G.canonicalize(raw);
  const ids = list.map((g) => g.id);
  assert.equal(new Set(ids).size, ids.length, 'ids are unique');
  for (const id of ids) assert.match(id, /^(dora|phase|pr|estimate|capacity)_[a-z0-9_]+$/);
  assert.ok(ids.includes('estimate_accuracy'));
  assert.ok(ids.includes('dora_no_deploy_tags'));
  assert.ok(ids.includes('pr_no_data'));
  assert.ok(ids.includes('dora_lead_time'));
  const dev = list.find((g) => g.id === 'phase_dev_weak');
  assert.equal(dev.severity, 'danger');
  assert.equal(dev.level, 'bad');
  assert.ok(dev.reason.includes('dev weak') && dev.reason.includes('n=1'));
  assert.equal(list.find((g) => g.id === 'dora_deploy_frequency').severity, 'danger');
  assert.ok(list.find((g) => g.id === 'dora_no_deploy_tags').tiles.includes('dora_lead_time'));
  assert.equal(list[0].severity, 'danger', 'danger first');
});

// Property: for ANY computed metric suite, every metric with n < MIN_N has a
// guardrail whose id (or tiles) is that metric's tile key.
test('property: every metric with n < min_n has a matching guardrail', () => {
  let seed = 7;
  const rnd = () => { seed = (seed * 1103515245 + 12345) % 2147483648; return seed / 2147483648; };
  const n = () => Math.floor(rnd() * 9); // 0..8 straddles MIN_N
  for (let i = 0; i < 300; i++) {
    const met = {
      flow: { throughputPerWeek: { n: n() }, wip: { n: n() }, estimateAccuracy: { n: n() }, cycleTimeByPhase: { n: n() }, contextSwitching: { n: n() }, unplannedShare: { n: n() } },
      phases: { design: { n: n() }, dev: { n: n() }, review_pickup: { n: n() }, qa_wait: { n: n() }, deploy: { n: n() } },
      dora: { deploy_frequency: { total: n() }, lead_time: { n: n() }, change_failure_rate: { total: n(), failed: 0 }, mttr: { n: n() } },
      pr_flow: { pickup_days: { n: n() }, review_days: { n: n() }, size: { n: n() }, merge_lag_days: { n: n() }, unreviewed: { n: n() } },
    };
    const envs = G.envelopesOf(met);
    assert.equal(Object.keys(envs).length, 6 + 5 + 4 + 5);
    const canon = G.canonicalize([...G.evaluate(met.flow, {}), ...G.lowNGuardrails(envs)]);
    assert.deepEqual(G.uncovered(envs, canon), [], `iteration ${i}`);
    // …and nothing flags a metric that has enough data.
    for (const g of canon.filter((x) => x.code === 'low_n')) {
      const k = Object.keys(envs).find((key) => G.tileOf(key) === g.id);
      assert.ok(k && envs[k].n < G.MIN_N, `${g.id} flagged without low n`);
    }
  }
});

test('tileOf maps metric keys onto tile prefixes', () => {
  assert.equal(G.tileOf('deploymentFrequency'), 'dora_deploy_frequency');
  assert.equal(G.tileOf('dora.dora_cfr'), 'dora_cfr');
  assert.equal(G.tileOf('phases.review_pickup'), 'phase_review_pickup_weak');
  assert.equal(G.tileOf('prPickup'), 'pr_pickup');
  assert.equal(G.tileOf('throughputPerWeek'), 'capacity_throughput');
});

// ---- guardFor + the coverage contract ---------------------------------------

const T_UNTIL = Date.UTC(2026, 0, 19);
const H = (n, extra = {}) => ({ value: 1, n, coverage: 1, weak_reasons: [], ...extra });
const Q = (n) => ({ p50: 1, p75: 2, n });

/** Sparse computeMetrics()-shaped fixture: n < MIN_N in EVERY family. */
function sparse() {
  return {
    window: { since: T_UNTIL - 14 * 86400000, until: T_UNTIL },
    dora: {
      deploy_frequency: { value: 0.5, per_week: 0.5, total: 1, by_repo: { r1: 1 }, per_week_by_repo: { r1: 0.5 }, by_kind: { regular: 0, hotfix: 1 } },
      lead_time: { value: 2, p50: 2, p75: 3, p85: 3, n: 2 },
      ticket_lead_time: { value: 3, n: 2 },
      change_failure_rate: { value: 0.5, failed: 1, total: 2, by_signal: { hotfix_tag: 1, bug: 0, jira_rework: 0, jira_reopened: 0 }, window_days: 7 },
      mttr: { value: 5, n: 2, incidents: 2, from_deploy: 2 },
      open_incidents: { n: 1 }, hotfix_rate: { value: 1, hotfix: 1, total: 1 }, batch_size: { n: 1, deploys_without_tickets: 0 }, time_to_first_commit: { n: 1 },
      deploys_per_capacity_week: { value: 0.2, n: 1 },
      guardrails: [{ n: 1, min_n: 3 }],
      bands: { deploy_frequency: [{ min: 1 }], lead_time_days: [{ max: 1 }], change_failure_rate: [{ max: 0.15 }], mttr_hours: [{ max: 24 }] },
      window: { start: 0, end: 1 },
    },
    flow: {
      throughputPerWeek: H(2, { value: { count: 2, weighted: 3, weeks: 2, count_per_week: 1, weighted_per_week: 1.5 }, per_person: { u1: { count: 2, weighted: 3, capacity_days: 3, time_off_days: 7, count_per_capacity_day: 0.6, weighted_per_capacity_day: 1 } } }),
      wip: H(1, { value: { count: 1 } }),
      agingWip: H(1, { value: { count: 1, p75_by_bucket: { Story: 4 } } }),
      contextSwitching: H(2, { value: 1.5, by_person: { u1: { active_days: 2, mean_keys_per_day: 1.5 } } }),
      focus: H(2, { value: 0.5, focused_days: 1, by_person: { u1: { active_days: 2, focused_days: 1, focus_share: 0.5 } } }),
      flowEfficiency: H(2, { value: 0.4, active_days: 2, elapsed_days: 5, median_ticket: 0.4 }),
      investmentMix: H(2, { value: { total_dev_days: 3, by_type: { Story: { days: 3, share: 1 } }, by_epic: { 'no epic': { days: 3, share: 1 } } } }),
      investmentByEpic: H(2, { value: { total_dev_days: 3, rows: [{ dev_days: 3, share: 1 }], no_epic: { dev_days: 0, share: 0 } } }),
      unplannedShare: H(2, { value: 0.5, unplanned: 1 }),
      escapeRate: H(2, { value: 0.5, escaped: 1 }),
      sprintPlanning: H(2, { value: { accuracy: 0.5, scope_creep: 0.5, sprints: [{ planned: 2, added: 1 }] } }),
      reworkRateByPerson: H(2, { value: { u1: { rate: 0.5, n: 2 } } }),
      estimateAccuracy: H(2, { value: { histogram: { '<0.5': 0, '0.5-0.8': 1, '0.8-1.25': 1, '1.25-2': 0, '>2': 0 } } }),
      cycleTimeByPhase: H(2, { value: { dev: { median: 2, p75: 3, n: 2, coverage: 1 } }, phase_n: { dev: 2 } }),
      deploysPerCapacityWeek: H(1, { value: 0.2 }),
    },
    capacity: { people: { u1: { business_days: 10, time_off_days: 7, capacity_days: 3 } }, business_days: 10, time_off_days: 7, capacity_days: 3, headcount: 1 },
    phase_population: 2,
    phases: { dev: { n: 2, p50: 2, p75: 3, coverage: 1 }, design: { n: 0, coverage: 0 } },
    // (healthy test bumps n; coverage is reset there)
    pr_flow: {
      pickup_days: Q(2), review_days: Q(2), merge_lag_days: Q(2), coding_days: Q(2), pr_cycle_days: Q(2), size: Q(2),
      review_depth: Q(2), comments_count: Q(2), reviewers_n: Q(2), post_review_commits: Q(2), review_rounds: Q(2),
      size_buckets: { '<50': 1, '<200': 1, '<400': 0, '>=400': 0 },
      unreviewed: { count: 1, share: 0.5, n: 2 }, merged_without_approval: { count: 0, share: 0, n: 2 }, reworked_after_review: { count: 0, share: 0, n: 2 },
      counts: { total: 2, merged: 2, declined: 0, open: 0 }, review_load: { n: 2, top_share: 1, reviewers_n: 1 },
      total: 2, approximated_times: 0, partial_count: 1, unreviewed_share: 0.5, merges_per_capacity_week: 0.6,
    },
    pr_sync: { state: 'ok', cursor_by_repo: { r1: { in_progress: true, synced_at: T_UNTIL - 5 * 86400000, prs: 2 } }, fetched: 2 },
    rework: { n: 2, rate_all: 0.5, rate_strong: 0.5, fix_rate: 0.5, jira_low_confidence_share: 0.75, rework_days: 1, dev_days: 4, items: [{ days: 1, confidence: 'low' }] },
    estimates: { calibration: { n: 2, factor: 1.1 }, inflation: { n: 2, n_ref: 1, index: 1.2 } },
    subtasks: [{ dev_days: 1 }],
    investment: { total_dev_days: 3 },
    estimate_accuracy: { histogram: { '<0.5': 0 } },
  };
}

test('contract: every numeric leaf of a sparse metrics object is guarded or whitelisted', () => {
  assert.deepEqual(G.unguardedPaths(sparse()), []);
});

test('contract: every covering tile actually raises a guard on the sparse fixture', () => {
  const met = sparse();
  const tiles = new Set(G.numericLeaves(met).map(G.coverageOf).filter((t) => t && t !== 'whitelist'));
  assert.ok(tiles.size >= 30, `fixture reaches ${tiles.size} tiles`);
  for (const t of tiles) {
    const g = G.guardFor(met, [t]);
    assert.notEqual(g.level, 'ok', `${t} is unguarded on a sparse scope`);
    assert.ok(g.reasons.length && g.reasons.every((r) => typeof r === 'string' && r.length < 80), t);
  }
});

test('contract: a new unguarded numeric key fails', () => {
  const met = sparse();
  met.flow.brandNewMetric = { value: 3, n: 1 };
  met.velocity_score = 7;
  assert.deepEqual(G.unguardedPaths(met).sort(), ['flow.brandNewMetric.n', 'flow.brandNewMetric.value', 'velocity_score']);
});

test('contract: every tile/metric key the module names resolves to a guard', () => {
  const keys = new Set([...Object.keys(G.TILE_OF), ...Object.values(G.AFFECTS).flat()]);
  for (const k of keys) {
    assert.ok(G.guardFor(sparse(), [k]).level !== 'ok', `${k} has no guard`);
  }
  for (const [, tile] of G.COVERAGE) assert.ok(G.TILE_GUARDS[tile], `COVERAGE → unknown tile ${tile}`);
});

test('contract: real computeMetrics output on a tiny scope is fully covered', () => {
  const M = require('../lib/metrics.js');
  const DAY = 86400000;
  const t0 = Date.UTC(2026, 0, 5, 9);
  const config = { workweek: [1, 2, 3, 4, 5], qa_work_min_commit_days: 2, timezone: 'UTC' };
  const recs = [{ key: 'ABC-1', type: 'Story', assignee_id: 'u1', status_category: 'done', status: 'Done', created: t0 - DAY, done_at: t0 + 3 * DAY, eff_done_at: t0 + 3 * DAY, first_active_at: t0, intervals: [{ status: 'In Progress', from: t0, to: t0 + 2 * DAY }], commit_ts: [t0 + DAY] }];
  const withPh = M.attachPhases(recs, { prMap: new Map(), tags: [], config, people: {}, canonical: (x) => x });
  const met = M.computeMetrics({ records: withPh, estimates: { 'ABC-1': { days: 2 } }, people: {}, flat_people: { u1: { name: 'P' } }, canonical: (x) => x, config },
    { since: t0, until: t0 + 14 * DAY },
    { tags: [{ name: 'deployed-1', ts: t0 + 4 * DAY }], prs: [{ key: 'ABC-1', opened_at: t0, merged_at: t0 + 2 * DAY, first_review_at: t0 + DAY, additions: 10, deletions: 2 }] });
  assert.deepEqual(G.unguardedPaths(met), []);
});

test('guardFor: short inline reasons per family', () => {
  const met = sparse();
  assert.deepEqual(G.guardFor(met, ['dora_deploy_frequency']), { level: 'bad', reasons: ['n=1, too few deploys', 'only hotfix/hf tags matched'] });
  assert.ok(G.guardFor(met, ['mttr']).reasons.includes('100% of restores timed from the deploy, not the incident'));
  const pr = G.guardFor(met, ['prPickup']).reasons;
  assert.ok(pr.includes('n=2, too few PRs'));
  assert.ok(pr.includes('PR sync in progress (1 repo)'));
  assert.ok(pr.includes('PR cache 5d old'));
  assert.ok(pr.includes('1 PR partially fetched'));
  assert.ok(G.guardFor(met, ['reworkRateAll']).reasons.includes('75% of Jira rework links low-confidence'));
  assert.ok(G.guardFor(met, ['estimate_inflation']).reasons.includes('n_ref=1, thin reference period'));
  assert.ok(G.guardFor(met, ['deploysPerCapacityWeek']).reasons.includes('only 3 capacity days'));
  assert.ok(G.guardFor(met, ['phases.design']).reasons.some((r) => r.startsWith('design: n=0')));
  assert.equal(G.guardFor({}, ['dora_lead_time']).level, 'bad', 'missing metric = no data');
  assert.deepEqual(G.guardFor(met, ['not_a_tile']), { level: 'ok', reasons: [] });
});

test('guardFor: a healthy scope reads ok', () => {
  const met = sparse();
  const big = (o) => { for (const v of Object.values(o)) if (v && typeof v === 'object') { if ('n' in v) v.n = 40; if ('total' in v) v.total = 40; big(v); } };
  big(met);
  met.dora.deploy_frequency.by_kind = { regular: 30, hotfix: 10 };
  met.dora.mttr.from_deploy = 3;
  met.pr_flow.partial_count = 0;
  met.pr_sync = { state: 'ok', cursor_by_repo: { r1: { in_progress: false, synced_at: T_UNTIL - 3600000 } } };
  met.rework.jira_low_confidence_share = 0.1;
  met.capacity.capacity_days = 180; met.capacity.business_days = 200;
  met.estimates.inflation.n_ref = 40;
  met.phases.design.coverage = 1;
  for (const t of Object.keys(G.TILE_GUARDS)) assert.deepEqual(G.guardFor(met, [t]), { level: 'ok', reasons: [] }, t);
});

test('evaluate: pr_cache_partial, jira_rework_low_confidence, hf_only_tags', () => {
  const list = G.evaluate(HEALTHY_M, { ...HEALTHY_IN, pr_cache: { in_progress: true, age_days: 4, partial_count: 2 }, jira_rework_low_confidence_share: 0.6, deploy_kinds: { regular: 0, hotfix: 3 } });
  assert.deepEqual(codes(list).sort(), ['hf_only_tags', 'jira_rework_low_confidence', 'pr_cache_partial']);
  const canon = G.canonicalize(list);
  assert.ok(canon.find((g) => g.id === 'pr_cache_partial').tiles.includes('pr_depth'));
  assert.ok(canon.find((g) => g.id === 'capacity_rework_low_confidence').tiles.includes('capacity_rework_rate_strong'));
  assert.ok(canon.find((g) => g.id === 'dora_hf_only_tags').tiles.includes('dora_deploys_per_capacity_week'));
  assert.deepEqual(G.evaluate(HEALTHY_M, { ...HEALTHY_IN, pr_cache: { in_progress: false, age_days: 1, partial_count: 0 }, jira_rework_low_confidence_share: 0.2, deploy_kinds: { regular: 2, hotfix: 1 } }), []);
});
