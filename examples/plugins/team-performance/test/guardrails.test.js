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
