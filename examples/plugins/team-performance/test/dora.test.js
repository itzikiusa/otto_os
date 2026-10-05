// DORA metrics — pure, deterministic fixtures.
// Run: node --test test/dora.test.js
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { doraMetrics, bandFor } = require('../lib/dora.js');

const T = (s) => Date.parse(s);

// 3 deploy tags (Mon / Wed / next Mon) in one repo; the Wed tag is a hotfix.
const tags = [
  { repo: 'svc', name: 'v1-deployed', ts: T('2026-06-01T10:00:00Z'), kind: 'regular' },
  { repo: 'svc', name: 'v1.1-hf', ts: T('2026-06-03T10:00:00Z'), kind: 'hotfix' },
  { repo: 'svc', name: 'v2-deployed', ts: T('2026-06-08T10:00:00Z'), kind: 'regular' },
];
const records = [
  { key: 'ABC-1', type: 'Story', first_commit_at: T('2026-05-28T10:00:00Z'), deployed_at: T('2026-06-01T10:00:00Z'), deployed_tag: 'v1-deployed' }, // Thu→Mon = 2 bd
  { key: 'ABC-2', type: 'Story', first_commit_at: T('2026-05-29T10:00:00Z'), deployed_at: T('2026-06-01T10:00:00Z'), deployed_tag: 'v1-deployed' }, // Fri→Mon = 1 bd
  { key: 'ABC-3', type: 'Task', first_commit_at: T('2026-06-02T10:00:00Z'), deployed_at: T('2026-06-03T10:00:00Z'), deployed_tag: 'v1.1-hf' }, // 1 bd
  { key: 'ABC-4', type: 'Story', first_commit_at: T('2026-06-01T10:00:00Z'), deployed_at: T('2026-06-08T10:00:00Z'), deployed_tag: 'v2-deployed' }, // 5 bd
  // Bug created 2 days after v2 shipped ABC-4, pointing back at it.
  { key: 'ABC-5', type: 'Bug', created: T('2026-06-10T10:00:00Z'), rework_of: ['ABC-4'] },
];
const window = { start: T('2026-06-01T00:00:00Z'), end: T('2026-06-15T00:00:00Z') }; // 2 weeks

test('doraMetrics: exact values on the deterministic fixture', () => {
  const m = doraMetrics({ records, tags, window, cfg: { failure_window_days: 7, min_n: 5 } });
  assert.equal(m.deploy_frequency.total, 3);
  assert.equal(m.deploy_frequency.per_week, 1.5);
  assert.deepEqual(m.deploy_frequency.by_repo, { svc: 3 });
  assert.equal(m.deploy_frequency.band, 'high');

  assert.equal(m.lead_time.n, 4);
  assert.equal(m.lead_time.p50, 1.5); // [1,1,2,5]
  assert.equal(m.lead_time.p75, 2.75);
  assert.equal(m.lead_time.band, 'high');

  // v1 failed (hotfix 2d later); v2 failed (bug on ABC-4 2d later); hf itself fine.
  assert.equal(m.change_failure_rate.failed, 2);
  assert.equal(m.change_failure_rate.total, 3);
  assert.equal(m.change_failure_rate.value, 0.67);
  assert.equal(m.change_failure_rate.band, 'low');
  assert.deepEqual(m.change_failure_rate.evidence.map((e) => e.deploy), ['v1-deployed', 'v2-deployed']);
  assert.equal(m.change_failure_rate.evidence[1].signals[0].ref, 'ABC-5');

  // v1 → hf = 48h; ABC-5's bug has no later deploy → not restored.
  assert.equal(m.mttr.n, 1);
  assert.equal(m.mttr.p50, 48);
  assert.equal(m.mttr.band, 'medium');

  const metrics = m.guardrails.map((g) => g.metric).sort();
  assert.deepEqual(metrics, ['change_failure_rate', 'deploy_frequency', 'lead_time', 'mttr']);
});

test('doraMetrics: same-day tags in one repo collapse into one deploy', () => {
  const t2 = [...tags, { repo: 'svc', name: 'v1b-deployed', ts: T('2026-06-01T15:00:00Z'), kind: 'regular' }];
  assert.equal(doraMetrics({ records: [], tags: t2, window }).deploy_frequency.total, 3);
});

test('doraMetrics: empty input → null values, never 0/NaN', () => {
  const m = doraMetrics({ records: [], tags: [], window });
  assert.equal(m.deploy_frequency.total, 0);
  assert.equal(m.lead_time.p50, null);
  assert.equal(m.lead_time.value, null);
  assert.equal(m.change_failure_rate.value, null);
  assert.equal(m.mttr.value, null);
  assert.equal(m.mttr.band, null);
  assert.equal(bandFor('lead_time_days', null), null);
});
