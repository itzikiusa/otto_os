// Unit tests for the report model + deterministic HTML renderer (pure, no I/O
// beyond reading the bundled report/ assets).
// Run (from the plugin dir): node --test test/reportmodel.test.js
const { test } = require('node:test');
const assert = require('node:assert/strict');

const RM = require('../lib/reportmodel.js');

const NAMES = ['Alice Example', 'Bob Sample', 'Carol Tester'];
const KEYS = ['ABC-1', 'ABC-22', 'XYZ-303', 'ABC-4040'];

function fixture(over = {}) {
  return {
    scope: 'Team + everyone',
    period: { start: '2026-07-01', end: '2026-09-30' },
    compare: { label: 'Q2', kpis: { throughput: 40 } },
    jiraBase: 'https://example.atlassian.net/',
    meta: { title: 'Q3 review', data_as_of: '2026-10-01T08:00:00Z', scan_time: '2026-10-01T07:00:00Z', ruler_version: 'r3', report_id: 'rep1', comments_endpoint: '/report/comments?id=rep1' },
    kpis: [
      { id: 'throughput', label: 'Tickets delivered', value: 46, better: 'up' },
      { id: 'pace', label: 'Points per capacity day', value: 0.8, kind: 'ratio', capacity_days: 120 },
    ],
    phases: {
      rows: [
        { phase: 'design', median_days: null, tickets_tracked: 0, tickets_total: 30 },
        { phase: 'dev', median_days: 3.5, p85_days: 8, total_days: 140, tickets_tracked: 30, tickets_total: 30 },
        { phase: 'review', median_days: 0.8, tickets_tracked: 25, tickets_total: 30 },
      ],
      tickets: [{ key: 'ABC-1', title: 'Payments for Alice Example', person: 'Alice Example', design: null, dev: 2, review: 0.5 }],
    },
    dora: { deployments: 12, deploy_frequency_per_week: 0.9, change_failure_rate: 0.17, failures: [{ key: 'ABC-22', kind: 'hotfix', caused_by: 'ABC-1', restore_hours: 5 }] },
    pr_flow: { prs: 50, pickup_hours_median: 6, by_person: [{ person: 'Bob Sample', prs: 20 }] },
    rework: { rate: 0.12, in: [{ key: 'ABC-1', by_key: 'XYZ-303', person: 'Alice Example', lines: 80 }], jira: [{ key: 'XYZ-303', source_key: 'ABC-1', reason: 'follow-up of ABC-1 by Bob' }] },
    flow: { wip_avg: 7, throughput: [{ week: '2026-W27', count: 4 }], investment: [{ label: 'Story', share: 0.6 }], estimate_accuracy: { median_ratio: 1.3, buckets: [{ label: '≤1×', count: 10 }] } },
    capacity: [{ person: 'Alice Example', working_days: 64, time_off_days: 4, capacity_days: 60, delivered_points: 40 }],
    substantive_subtasks: [{ person: 'Carol Tester', key: 'ABC-4040', parent_key: 'ABC-1', parent_owner: 'Alice Example', dev_days: 2, commits: 5, reason: 'own commits' }],
    guardrails: [{ level: 'warn', metric: 'estimates', message: 'Only 40% of tickets have an estimate for Bob Sample.' }],
    people: NAMES.map((name, i) => ({ name, role: 'Engineer', capacity_days: 60, design_days: null, tickets: [{ key: KEYS[i], title: `Work by ${name}` }], notes: [`Paired with ${NAMES[(i + 1) % 3]} on ${KEYS[3]}`] })),
    ...over,
  };
}
const NARR = { summary: 'Alice Example shipped ABC-1 and Bob fixed XYZ-303.', strengths: ['Carol Tester reviews fast'], goals: ['Cut pickup on ABC-22'] };

test('model: every section key is present, even from empty input', () => {
  for (const input of [fixture(), {}]) {
    const m = RM.buildReportModel(input);
    for (const k of RM.SECTION_KEYS) assert.ok(k in m, `missing ${k}`);
    assert.equal(m.phases.rows.length, 5);
    assert.ok(m.glossary.length >= RM.DEFAULT_GLOSSARY.length);
  }
});

test('model: null design stays null (not 0) and renders as "not tracked"', () => {
  const m = RM.buildReportModel(fixture());
  const design = m.phases.rows.find((r) => r.phase === 'design');
  assert.equal(design.median_days, null);
  assert.equal(design.not_tracked, true);
  const html = RM.renderReport(m, NARR);
  const phases = html.slice(html.indexOf('id="phases"'), html.indexOf('id="dora"'));
  assert.match(phases, /<td>Design<\/td><td class="n"><span class="nt">not tracked<\/span>/);
  assert.match(phases, /<text class="lbl"[^>]*>not tracked<\/text>/);
});

test('model: kpi delta vs compare, ratio keeps capacity', () => {
  const m = RM.buildReportModel(fixture());
  assert.equal(m.kpis[0].delta, 6);
  assert.equal(m.kpis[1].capacity_days, 120);
  const html = RM.renderReport(m, {});
  assert.match(html, /over 120 capacity days/);
});

test('render: unmasked keys link to jiraBase/browse/KEY', () => {
  const html = RM.renderReport(RM.buildReportModel(fixture()), NARR);
  assert.match(html, /href="https:\/\/example\.atlassian\.net\/browse\/ABC-1"/);
  assert.match(html, /href="https:\/\/example\.atlassian\.net\/browse\/XYZ-303"/);
  assert.match(html, /<li><strong>estimates<\/strong>/);
});

test('render: masked output has no links and no planted names or keys', () => {
  const m = RM.buildReportModel(fixture({ mask: true }));
  assert.equal(m.meta.masked, true);
  assert.equal(m.meta.jira_base, null);
  const html = RM.renderReport(m, NARR);
  assert.doesNotMatch(html, /\/browse\//);
  assert.deepEqual(RM.leakCheck(html, [...NAMES, ...NAMES.map((n) => n.split(' ')[0]), ...KEYS, 'example.atlassian']), []);
  assert.doesNotMatch(html.replace(/<script[\s\S]*<\/script>/, ''), /\b[A-Z][A-Z0-9]+-\d+\b/);
  assert.match(html, /Person A/);
  assert.match(html, /Ticket 1/);
});

test('render: masked render of a re-hydrated (JSON) model still drops keys from narrative', () => {
  const m = JSON.parse(JSON.stringify(RM.buildReportModel(fixture({ mask: true }))));
  const html = RM.renderReport(m, { summary: 'See ABC-999' });
  assert.deepEqual(RM.leakCheck(html, ['ABC-999']), []);
});

test('render: self-contained, a single nonce script, deterministic', () => {
  const m = RM.buildReportModel(fixture());
  const a = RM.renderReport(m, NARR);
  const b = RM.renderReport(RM.buildReportModel(fixture()), NARR);
  assert.equal(a, b);
  const scripts = a.match(/<script\b[^>]*>/g);
  assert.equal(scripts.length, 1);
  const nonce = scripts[0].match(/nonce="([^"]+)"/)[1];
  assert.ok(a.includes(`script-src 'nonce-${nonce}'`));
  assert.doesNotMatch(a, /<link\b|src="http/);
  assert.doesNotMatch(a, /\{\{[A-Z_]+\}\}/);
  assert.match(a, /prefers-color-scheme: dark/);
  assert.match(a, /\[data-theme='dark'\]/);
  assert.match(a, /@media print/);
  // every section has a How-to-read note except the glossary
  for (const id of ['kpis', 'phases', 'dora', 'pr_flow', 'rework', 'flow', 'capacity', 'substantive_subtasks', 'people']) {
    const start = a.indexOf(`<section class="card" id="${id}"`);
    assert.ok(start > 0, id);
    assert.ok(a.slice(start, a.indexOf('</section>', start)).includes('How to read it'), id);
  }
});

test('render: charts are accessible with title/desc and a table fallback', () => {
  const html = RM.renderReport(RM.buildReportModel(fixture()), {});
  const svgs = html.match(/<svg viewBox[^>]*role="img"[^>]*>/g) || [];
  assert.ok(svgs.length >= 3);
  assert.equal((html.match(/<summary>Show as table<\/summary>/g) || []).length, svgs.length);
  assert.equal((html.match(/<title id="ch-/g) || []).length, svgs.length);
});

test('render: script-breaking strings in data are neutralised', () => {
  const html = RM.renderReport(RM.buildReportModel(fixture({ scope: '</script><script>alert(1)</script>' })), { summary: '<img src=x onerror=alert(1)>' });
  assert.equal((html.match(/<script\b/g) || []).length, 1);
  assert.doesNotMatch(html, /<img src=x/);
});

test('model: comments endpoint must be a same-origin path, otherwise read-only', () => {
  assert.equal(RM.buildReportModel(fixture()).meta.comments_endpoint, '/report/comments?id=rep1');
  assert.equal(RM.buildReportModel(fixture({ meta: { comments_endpoint: 'https://evil.example/x' } })).meta.comments_endpoint, null);
});

test('model: derived guardrails flag weak inputs', () => {
  const m = RM.buildReportModel({ kpis: [{ id: 'r', label: 'Ratio', value: 1, kind: 'ratio' }] });
  const metrics = m.guardrails.map((g) => g.metric);
  assert.ok(metrics.includes('kpis.capacity'));
  assert.ok(metrics.includes('pr_flow'));
  assert.ok(metrics.includes('dora'));
  assert.ok(metrics.includes('phases.design'));
  const html = RM.renderReport(m, {});
  assert.match(html, /class="banner warn" role="alert"[^>]*><svg class="ico"/);
  assert.match(html, /capacity unknown/);
});
