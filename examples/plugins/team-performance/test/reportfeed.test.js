// Unit tests for lib/reportfeed.js (pure report extras).
// Run (from the plugin dir): node --test test/reportfeed.test.js
const { test } = require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');

const RF = require('../lib/reportfeed.js');

const kp = (vals) => [
  { id: 'tickets', label: 'Tickets delivered', value: vals[0], kind: 'count', better: 'up' },
  { id: 'cycle', label: 'Dev time median', value: vals[1], unit: 'd', kind: 'days', better: 'down' },
];

function fixture(over = {}) {
  return {
    scope: 'team',
    metricsByPeriod: [
      { label: 'P1', kpis: kp([10, 5]), phases: { rows: [{ phase: 'dev', median_days: 4, tickets_tracked: 8, tickets_total: 10 }] } },
      { label: 'P2', kpis: kp([12, 4]) },
      { label: 'P3', kpis: kp([14, 3]) },
    ],
    current: {
      label: 'P4',
      period: { start: '2026-07-01', end: '2026-10-01' },
      kpis: kp([11, 2]),
      phases: { rows: [{ phase: 'dev', median_days: 2, tickets_tracked: 3, tickets_total: 3 }] },
      records: [
        { key: 'ABC-1', summary: 'Checkout flow redesign', assignee_name: 'Alice Example', dev_days: 9, rework_days: 3, estimate_days: 2, actual_days: 9 },
        { key: 'ABC-2', summary: 'Login timeout fix', assignee_name: 'Bob Sample', dev_days: 1, estimate_days: 1, actual_days: 1, rework_of: 'ABC-1' },
        { key: 'ABC-3', summary: 'Report export', assignee_name: 'Alice Example', dev_days: 4, rework_days: 1, estimate_days: 8, actual_days: 4 },
      ],
      tags: [
        { name: 'release-deployed-1', ts: '2026-07-10T10:00:00Z' },
        { name: 'HF-1.2.1', ts: '2026-08-01T10:00:00Z' },
        { name: 'v1.3.0', ts: '2026-08-02T10:00:00Z' },
        { name: 'deployed-old', ts: '2026-06-01T10:00:00Z' },
      ],
      dora: { failures: [{ key: 'ABC-2', tag: 'HF-1.2.1', kind: 'hotfix', caused_by: 'ABC-1', restore_hours: 5 }] },
      rework: { jira: [{ key: 'ABC-2', source_key: 'ABC-1', title: 'Login timeout fix', reason: 'caused-by', points: 1 }] },
    },
    reworkResult: { pairs: { 'ABC-2>ABC-1': { lines: 30 }, 'ABC-3>ABC-1': { lines: 10 }, 'ABC-1>ABC-3': { lines: 5, days: 0.7 } } },
    prs: [{ id: 7, key: 'ABC-1', title: 'Checkout flow redesign', pickup_days: 0.5, review_days: 1.25, merge_lag_days: 0.1, size: 420, comments_count: 6 }],
    guardrails: [
      { id: 'low_n_cycle', level: 'warn', metric: 'cycle', message: 'cycle: n=3 < 5; indicative only' },
      { id: 'pr_times', level: 'warn', metric: 'prPickup', message: '2 PR(s) have open/merge times approximated' },
      { id: 'design', level: 'warn', metric: 'design', message: 'No design evidence on any ticket' },
      { id: 'other', level: 'warn', metric: 'x', message: 'irrelevant' },
    ],
    ...over,
  };
}

test('PR hours are derived from day values (×24)', () => {
  const out = RF.buildReportExtras(fixture());
  assert.deepEqual(out.pr_flow_items[0], { id: 7, key: 'ABC-1', title: 'Checkout flow redesign', pickup_hours: 12, review_hours: 30, merge_hours: 2.4, size: 420, comments: 6 });
  assert.equal(out.slow_prs[0].total_hours, 44.4);
  assert.equal(RF.daysToHours(null), null);
});

test('trend covers prior periods + current; masked trend is numeric only', () => {
  const out = RF.buildReportExtras(fixture());
  const t = out.trend.find((x) => x.id === 'tickets');
  assert.deepEqual(t.points.map((p) => p.value), [10, 12, 14, 11]);
  assert.equal(t.better, 'up');
  const m = RF.buildReportExtras(fixture({ masked: true }));
  assert.deepEqual(m.trend.find((x) => x.id === 'cycle').points, [5, 4, 3, 2]);
  assert.equal(RF.buildReportExtras(fixture({ metricsByPeriod: [] })).trend.length, 0);
});

test('improved / declined from KPI deltas honour `better`', () => {
  const out = RF.buildReportExtras(fixture());
  assert.deepEqual(out.improved.map((x) => x.id), ['cycle']);
  assert.deepEqual(out.declined.map((x) => x.id), ['tickets']);
  assert.equal(out.declined[0].delta, -3);
  assert.ok(out.narrative.some((l) => l.startsWith('Improved:')));
});

test('outliers: dev days, rework, estimate miss, jira rework', () => {
  const out = RF.buildReportExtras(fixture());
  assert.equal(out.outliers.dev_days[0].key, 'ABC-1');
  assert.equal(out.outliers.dev_days[0].person, 'Alice Example');
  assert.equal(out.outliers.rework_days[0].value, 3);
  assert.equal(out.outliers.rework_lines[0].key, 'ABC-1');
  assert.equal(out.outliers.rework_lines[0].value, 40);
  assert.deepEqual(out.outliers.estimate_miss.map((x) => x.key), ['ABC-1', 'ABC-3']);
  assert.equal(out.outliers.jira_rework[0].source_key, 'ABC-1');
  assert.ok(out.outliers.dev_days.length <= RF.TOP_N);
});

test('rework pairs use charged days or apportion the origin days by lines', () => {
  const out = RF.buildReportExtras(fixture());
  const p = (by, key) => out.rework_pairs.find((x) => x.by_key === by && x.key === key);
  assert.equal(p('ABC-2', 'ABC-1').days, 2.25);
  assert.equal(p('ABC-3', 'ABC-1').days, 0.75);
  assert.equal(p('ABC-2', 'ABC-1').days_basis, 'apportioned_by_lines');
  assert.equal(p('ABC-1', 'ABC-3').days, 0.7);
  assert.equal(p('ABC-1', 'ABC-3').days_basis, 'charged');
});

test('dora failures get a title; deploy tags are filtered to the period and hotfix-flagged', () => {
  const out = RF.buildReportExtras(fixture());
  assert.equal(out.dora_failures[0].title, 'Login timeout fix');
  assert.deepEqual(out.deploy_tags.map((t) => [t.name, t.hotfix]), [['release-deployed-1', false], ['HF-1.2.1', true]]);
});

test('blind spots come from guardrails', () => {
  const out = RF.buildReportExtras(fixture());
  assert.deepEqual(out.blind_spots.map((b) => b.kind), ['low_coverage', 'approximated_pr_times', 'no_design_evidence']);
});

test('masking drops titles and aliases persons stably', () => {
  const out = RF.buildReportExtras(fixture({ masked: true }));
  const json = JSON.stringify(out);
  for (const s of ['Alice Example', 'Bob Sample', 'Checkout flow redesign', 'Login timeout fix', 'Report export', 'P1', 'P4']) assert.ok(!json.includes(s), s);
  assert.equal(out.outliers.dev_days[0].person, 'Person A');
  assert.ok(out.rework_pairs.every((p) => p.title === ''));
  assert.equal(out.dora_failures[0].caused_by, '');
});

test('deterministic: same input → identical output', () => {
  assert.deepEqual(RF.buildReportExtras(fixture()), RF.buildReportExtras(fixture()));
});

test('assignee file hint is sha256(assignee).slice(0,12)', () => {
  assert.equal(RF.assigneeFileHint('acc-1'), crypto.createHash('sha256').update('acc-1').digest('hex').slice(0, 12));
  assert.match(RF.assigneeFileHint('acc-1'), /^[0-9a-f]{12}$/);
});
