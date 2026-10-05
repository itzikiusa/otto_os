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
  assert.match(phases, /Not tracked: Design/);
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
  assert.match(html, /<li id="gr-\d+"><strong>estimates<\/strong>/);
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

// ---------------------------------------------------------------- round 2
function rich(over = {}) {
  return fixture({
    report_kind: 'team',
    trend: [
      { id: 'throughput', label: 'Tickets delivered', better: 'up', points: [{ label: 'Q4', value: 30 }, { label: 'Q1', value: 35 }, { label: 'Q2', value: 40 }, { label: 'Q3', value: 46 }] },
      { id: 'lead_time', label: 'Lead time', unit: 'd', better: 'down', points: [{ label: 'Q2', value: 6 }, { label: 'Q3', value: 4 }] },
    ],
    estimate_basis: { est_inflation: 1.2, pace_raw: 1.5, pace_adjusted: 1.8, pace_ref: 1.6, comparable_n: 25, ref_label: 'Q2' },
    phases: {
      rows: [
        { phase: 'design', median_days: null, tickets_tracked: 0, tickets_total: 30 },
        { phase: 'dev', median_days: 3.5, p85_days: 8, total_days: 140, tickets_tracked: 30, tickets_total: 30 },
        { phase: 'review', median_days: 0.8, tickets_tracked: 10, tickets_total: 30 },
        { phase: 'deployment', median_days: 1, tickets_tracked: 20, tickets_total: 30 },
        { phase: 'rework', median_days: 0.5, tickets_tracked: 16, tickets_total: 30 },
      ],
      tickets: [
        { key: 'ABC-1', title: 'Payments for Alice Example', person: 'Alice Example', dev: 9, review: 0.5, rework: 2, estimate_days: 3 },
        { key: 'ABC-22', title: 'Login', person: 'Bob Sample', dev: 2, review: 1, estimate_days: 2 },
      ],
    },
    dora: { deployments: 12, deploy_frequency_per_week: 0.9, change_failure_rate: 0.17, mttr_hours_median: 5, weekly: [{ week: '2026-07-06', deployments: 1 }, { week: '2026-07-13', deployments: 2 }], failures: [{ key: 'ABC-22', kind: 'hotfix', caused_by: 'ABC-1', restore_hours: 5 }, { key: 'XYZ-303', kind: 'bug', restore_hours: 9 }] },
    pr_flow: { prs: 50, pickup_hours_median: 30, items: [{ key: 'ABC-1', pickup_hours: 2, review_hours: 5 }, { pickup_hours: 40, review_hours: 1 }, { pickup_hours: 400 }] },
    flow: {
      wip_avg: 7,
      wip_per_person: 2.5,
      investment: [{ label: 'Story', share: 0.6 }],
      investment_by_epic: [{ epic_key: 'ABC-4040', epic_title: 'Checkout epic for Bob Sample', share: 0.4, points: 12 }],
      unplanned_share: 0.35,
      unplanned: [{ key: 'XYZ-303', title: 'Prod bug', type: 'Bug', person: 'Carol Tester', reason: 'hotfix after ABC-1' }],
      estimate_accuracy: { median_ratio: 1.3, buckets: [{ label: '≤1×', count: 3 }, { label: '>1×', count: 4 }] },
    },
    guardrails: [{ severity: 'info', metric: 'rework.blame', message: 'Blame window is 30 days.' }, { severity: 'danger', metric: 'dora.tags', message: 'Tags missing for Bob Sample repos.' }],
    goals: [{ goal: 'Cut pickup on ABC-22', target: '< 8 h', actual: '30 h', met: false }],
    ...over,
  });
}
const NEW_SECTIONS = ['trend', 'estimate_basis', 'outliers', 'next_steps', 'blind_spots'];
const sectionHtml = (html, id) => {
  const start = html.indexOf(`<section class="card" id="${id}"`);
  return start < 0 ? '' : html.slice(start, html.indexOf('</section>', start));
};

test('model: no all-null new section when the source data is present', () => {
  const m = RM.buildReportModel(rich());
  assert.equal(m.trend.length, 2);
  assert.equal(m.kpis[0].history.length, 4);
  assert.equal(m.estimate_basis.est_inflation, 1.2);
  assert.ok(m.outliers.dev.length >= 2 && m.outliers.dev[0].key === 'ABC-1');
  assert.equal(m.outliers.estimate[0].key, 'ABC-1');
  assert.equal(m.outliers.estimate[0].value, 3);
  assert.ok(m.outliers.rework.length && m.outliers.jira_linked.length);
  assert.equal(m.dora.failed_deployments, 2);
  assert.equal(m.dora.incidents, 2);
  assert.equal(m.people[0].phase_days.dev, 9);
  assert.ok(m.next_steps.some((s) => /pickup/i.test(s)));
  assert.ok(m.next_steps.some((s) => /inflated/i.test(s)));
  assert.ok(m.blind_spots.length >= 3);
  const html = RM.renderReport(m, NARR);
  for (const id of NEW_SECTIONS) {
    const sec = sectionHtml(html, id);
    assert.ok(sec, `section ${id} rendered`);
    assert.ok(sec.includes('How to read it'), id);
  }
  assert.match(sectionHtml(html, 'kpis'), /class="spark"/);
  assert.match(sectionHtml(html, 'trend'), /improving/);
  assert.match(sectionHtml(html, 'estimate_basis'), /×1\.2/);
  assert.match(sectionHtml(html, 'dora'), /2 of 12 deployments failed/);
  assert.match(sectionHtml(html, 'dora'), /median of 2 incidents/);
  assert.match(sectionHtml(html, 'dora'), /pill band low/);
  assert.match(sectionHtml(html, 'pr_flow'), /id="ch-pr-strip"/);
  assert.match(sectionHtml(html, 'rework'), /class="rflow"/);
  assert.match(sectionHtml(html, 'flow'), /Investment by epic/);
  assert.match(sectionHtml(html, 'flow'), /Unplanned work<\/h3>/);
  assert.match(sectionHtml(html, 'people'), /id="ch-people-phases"/);
  assert.match(sectionHtml(html, 'substantive_subtasks'), /Carol Tester <span class="muted small">· 1 sub-task/);
  assert.equal(sectionHtml(html, 'goals'), '', 'team reports have no Goals section');
});

test('render: fixed phase colours, direct segment labels, data in <desc>', () => {
  const html = RM.renderReport(RM.buildReportModel(rich({ phases: { rows: [], by_period: [{ label: 'Q3', design: 1, dev: 10, review: 2, deployment: 1, rework: 0.2 }] } })), {});
  const fig = html.slice(html.indexOf('id="ch-phases"'), html.indexOf('</figure>', html.indexOf('id="ch-phases"')));
  for (const [cls] of [['s4'], ['s1'], ['s3'], ['s6'], ['s5']]) assert.match(fig, new RegExp(`<rect class="${cls}"`));
  assert.match(fig, /<span class="hc-seg"[^>]*>10<\/span>/, 'wide dev segment labelled in place');
  assert.doesNotMatch(fig, /<span class="hc-seg"[^>]*>0\.2<\/span>/, 'narrow rework segment not labelled');
  assert.match(fig, /<desc id="ch-phases-d">[^<]*Q3: Design 1 d, Dev 10 d/);
  assert.doesNotMatch(fig.slice(fig.indexOf('<svg'), fig.indexOf('</svg>')), /<text/, 'no text inside the stretched SVG');
});

test('guardrails: section pills by prefix, info severity mapped, derived small-sample rules', () => {
  const m = RM.buildReportModel(rich());
  const by = Object.fromEntries(m.guardrails.map((g) => [g.metric, g.level]));
  assert.equal(by['rework.blame'], 'info');
  assert.equal(by['dora.tags'], 'error');
  assert.equal(by['phases.review'], 'warn', 'coverage < 0.5');
  assert.equal(by['flow.estimate_accuracy'], 'warn', 'histogram < 10');
  const small = RM.buildReportModel({ pr_flow: { prs: 3 }, dora: { deployments: 2 } });
  assert.ok(small.guardrails.some((g) => g.metric === 'pr_flow.sample'));
  assert.ok(small.guardrails.some((g) => g.metric === 'dora.sample'));
  const html = RM.renderReport(m, {});
  assert.match(sectionHtml(html, 'dora'), /<a class="pill gr error" href="#gr-\d+"/);
  assert.match(sectionHtml(html, 'rework'), /<a class="pill gr info"/);
  assert.match(sectionHtml(html, 'phases'), /<a class="pill gr (warn|info)"/);
  assert.match(html, /class="banner info" role="note"/);
  assert.match(html, /<li id="gr-0">/);
});

test('dev report: Goals section + team-context pill on DORA and PR flow', () => {
  const html = RM.renderReport(RM.buildReportModel(rich({ report_kind: 'dev' })), NARR);
  const goals = sectionHtml(html, 'goals');
  assert.match(goals, /not met/);
  assert.match(goals, /Proposed for next period/);
  assert.doesNotMatch(sectionHtml(html, 'summary'), /<h3>Goals<\/h3>/);
  assert.match(sectionHtml(html, 'dora'), /Team context \(not individual\)/);
  assert.match(sectionHtml(html, 'pr_flow'), /Team context \(not individual\)/);
  assert.doesNotMatch(sectionHtml(RM.renderReport(RM.buildReportModel(rich()), {}), 'dora'), /Team context/);
});

test('toc + anchors: every rendered section is in the sticky TOC with a link anchor', () => {
  const html = RM.renderReport(RM.buildReportModel(rich()), NARR);
  const toc = html.slice(html.indexOf('<nav class="toc'), html.indexOf('</nav>', html.indexOf('<nav class="toc')));
  for (const id of ['summary', 'kpis', 'trend', 'phases', 'outliers', 'dora', 'next_steps', 'blind_spots', 'glossary']) {
    assert.match(toc, new RegExp(`href="#${id}" data-toc="${id}"`));
    assert.match(html, new RegExp(`<a class="anchor" href="#${id}"`));
  }
});

test('masking: every new section is masked (names, keys, epic titles)', () => {
  const m = RM.buildReportModel(rich({ mask: true, report_kind: 'dev' }));
  const html = RM.renderReport(m, NARR, { comments: [{ anchor: 'dora', text: 'Ask Bob Sample about ABC-22', author: 'Alice Example', at: 1759651200000 }] });
  assert.deepEqual(RM.leakCheck(html, [...NAMES, ...NAMES.map((n) => n.split(' ')[0]), ...KEYS, 'XYZ-303', 'Checkout epic', 'Prod bug', 'example.atlassian']), []);
  assert.doesNotMatch(html.replace(/<script[\s\S]*<\/script>/, ''), /\b[A-Z][A-Z0-9]+-\d+\b/);
  for (const id of [...NEW_SECTIONS, 'goals', 'flow', 'rework', 'people']) assert.ok(sectionHtml(html, id), id);
  assert.match(sectionHtml(html, 'outliers'), /Ticket \d+/);
  assert.match(sectionHtml(html, 'flow'), /Ticket \d+/);
});

test('leak check: a planted key in the narrative is caught unmasked and scrubbed when masked', () => {
  const planted = 'QQQ-777';
  const narr = { summary: `See ${planted} for details`, goals: [`Close ${planted}`] };
  assert.deepEqual(RM.leakCheck(RM.renderReport(RM.buildReportModel(rich()), narr), [planted]), [planted]);
  assert.deepEqual(RM.leakCheck(RM.renderReport(RM.buildReportModel(rich({ mask: true })), narr), [planted]), []);
});

test('XSS: hostile strings in every new field are escaped', () => {
  const x = '<img src=x onerror=alert(1)>';
  const html = RM.renderReport(
    RM.buildReportModel(
      rich({
        report_kind: 'dev',
        trend: [{ id: 't', label: x, points: [{ label: x, value: 1 }, { label: 'b', value: 2 }] }],
        estimate_basis: { est_inflation: 1.3, note: x, ref_label: x },
        next_steps: [x],
        blind_spots: [x],
        goals: [{ goal: x, target: x, actual: x, met: true }],
        flow: { unplanned: [{ key: 'ABC-1', reason: x, type: x }], investment_by_epic: [{ epic_key: 'ABC-1', epic_title: x }] },
        outliers: { dev: [{ key: 'ABC-1', title: x, note: x, value: 1 }] },
        guardrails: [{ level: 'warn', metric: x, message: x }],
      }),
    ),
    { summary: x },
    { comments: [{ anchor: 'kpis"><script>', text: x, author: x, label: x, at: 'nope' }] },
  );
  assert.doesNotMatch(html, /<img src=x/);
  assert.equal((html.match(/<script\b/g) || []).length, 1);
  assert.match(html, /&lt;img src=x/);
});

test('comments: host-rendered with counts; at as epoch ms, numeric string or ISO', () => {
  const m = RM.buildReportModel(rich());
  const html = RM.renderReport(m, {}, {
    comments: [
      { anchor: 'dora', text: 'first', at: 1759651200000 },
      { anchor: 'dora', text: 'second', at: '1759737600000' },
      { anchor: 'kpis', text: 'third', at: '2026-10-07T10:00:00Z' },
      { anchor: 'kpis', text: '   ' },
    ],
  });
  assert.match(html, /<span id="c-count">3<\/span>/);
  assert.match(sectionHtml(html, 'dora'), /data-for="dora"[^>]*>Comments \(2\)</);
  assert.match(html, /2025-10-05 08:00/);
  assert.match(html, /2025-10-06 08:00/);
  assert.match(html, /<section class="card print-only" id="comments"/);
  assert.match(html, /role="dialog" aria-modal="true" aria-labelledby="c-title" hidden/);
});

test('palette: single source — no hex colour in report.css; light/dark/print generated', () => {
  const fs = require('fs');
  const path = require('path');
  const css = fs.readFileSync(path.join(__dirname, '..', 'report', 'report.css'), 'utf8');
  assert.doesNotMatch(css, /#[0-9a-fA-F]{3,8}(?![\w-])/);
  assert.doesNotMatch(css, /rgba?\(/);
  const pc = RM.paletteCss();
  for (const k of Object.keys(RM.PALETTE.light)) assert.ok(pc.includes(`--${k}: ${RM.PALETTE.light[k]}`), k);
  assert.match(pc, /prefers-color-scheme: dark\) \{ :root:not\(\[data-theme='light'\]\)/);
  assert.match(pc, /:root\[data-theme='dark'\]/);
  assert.match(pc, /@media print/);
  assert.equal(RM.PALETTE.light['cat-1'], '#1f6fd1');
  assert.equal(RM.PALETTE.dark['cat-4'], '#b18cff');
});

test('jira base: https only', () => {
  const { cleanBase } = RM._internal;
  assert.equal(cleanBase('https://example.atlassian.net/'), 'https://example.atlassian.net');
  assert.equal(cleanBase('http://example.atlassian.net'), null);
  assert.equal(cleanBase('javascript:alert(1)'), null);
  assert.equal(cleanBase('https://ex.com/"onmouseover=x'), null);
  assert.equal(RM.buildReportModel(fixture({ jiraBase: 'http://example.atlassian.net' })).meta.jira_base, null);
});

test('DORA bands follow the published thresholds', () => {
  const B = RM._internal.DORA_BANDS;
  assert.equal(B.deploy(8), 'elite');
  assert.equal(B.deploy(1), 'high');
  assert.equal(B.deploy(0.1), 'low');
  assert.equal(B.lead(0.5), 'elite');
  assert.equal(B.cfr(0.12), 'medium');
  assert.equal(B.mttr(200), 'low');
});

test('empty input still renders every always-on section without throwing', () => {
  const html = RM.renderReport(RM.buildReportModel({}), {});
  for (const id of ['kpis', 'trend', 'phases', 'outliers', 'dora', 'blind_spots']) assert.ok(sectionHtml(html, id), id);
  assert.match(sectionHtml(html, 'trend'), /no trend yet/);
});
