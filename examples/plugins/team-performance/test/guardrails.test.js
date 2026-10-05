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
