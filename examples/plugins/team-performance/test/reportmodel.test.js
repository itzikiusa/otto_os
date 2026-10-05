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

const NONCE_RE = /nonce(=|-)("?)[A-Za-z0-9]+/g;
test('render: self-contained, a single nonce script, deterministic apart from the nonce', () => {
  const m = RM.buildReportModel(fixture());
  const a = RM.renderReport(m, NARR);
  const b = RM.renderReport(RM.buildReportModel(fixture()), NARR);
  assert.equal(a.replace(NONCE_RE, 'nonce$1$2N'), b.replace(NONCE_RE, 'nonce$1$2N'));
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
  const figs = html.match(/<figure class="chart[\s\S]*?<\/figure>/g) || [];
  assert.ok(figs.length >= 3);
  for (const f of figs) {
    const imgs = f.match(/role="img" aria-labelledby="([^"]+)"/g) || [];
    assert.equal(imgs.length, 1, 'exactly one accessible image per chart');
    const [tid, did] = /aria-labelledby="([^" ]+) ([^"]+)"/.exec(f).slice(1);
    assert.ok(f.includes(`id="${tid}"`) && f.includes(`id="${did}"`), 'name + description are in the figure');
    assert.match(f, /<summary>Show as table<\/summary><div class="tbl-wrap"><table>/, 'table fallback is always present');
  }
  // horizontal charts: decorative row SVGs, label in HTML before its bar
  const h = figs.find((f) => f.includes('hchart'));
  assert.match(h, /<div class="hc-row"><div class="hc-l"[^>]*>[^<]+<\/div><div class="hc-track"><svg class="hc-svg"[^>]*aria-hidden="true"/);
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
  assert.match(fig, /<span class="sr-only" id="ch-phases-d">[^<]*Q3: Design 1 d, Dev 10 d/);
  for (const svg of fig.match(/<svg[\s\S]*?<\/svg>/g)) assert.doesNotMatch(svg, /<text/, 'no text inside a stretched SVG');
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
  const raw = fs.readFileSync(path.join(__dirname, '..', 'report', 'report.css'), 'utf8');
  assert.match(raw, /\/\* tp:fallback-start \*\/\s*:where\(:root\)/, 'fallback block is zero-specificity');
  const css = raw.replace(/\/\* tp:fallback-start \*\/[\s\S]*?\/\* tp:fallback-end \*\//, '');
  assert.doesNotMatch(css, /#[0-9a-fA-F]{3,8}(?![\w-])/);
  assert.doesNotMatch(css, /rgba?\(/);
  assert.deepEqual(RM.PALETTE.light, require('../lib/theme.js').light, 'palette is lib/theme.js');
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

// ---------------------------------------------------------------- round 3 extras
test('changes: improved / declined derived from KPIs with a prior and a direction', () => {
  const m = RM.buildReportModel(fixture({
    compare: { label: 'Q2', kpis: { throughput: 40, lead: 5, flat: 10 } },
    kpis: [
      { id: 'throughput', label: 'Tickets delivered', value: 46, better: 'up' },
      { id: 'lead', label: 'Lead time', value: 7, unit: 'd', better: 'down' },
      { id: 'flat', label: 'Flat one', value: 10.1, better: 'up' },
      { id: 'nodir', label: 'No direction', value: 3, prior: 1 },
    ],
  }));
  assert.deepEqual(m.changes.improved.map((x) => x.label), ['Tickets delivered']);
  assert.deepEqual(m.changes.declined.map((x) => x.label), ['Lead time']);
  const html = sectionHtml(RM.renderReport(m, {}), 'changes');
  assert.match(html, /<span class="delta better">\+6 \(\+15%\)<\/span>/);
  assert.match(html, /<span class="delta worse">\+2 d \(\+40%\)<\/span>/);
  assert.doesNotMatch(html, /Flat one|No direction/);
  // explicit lists win
  const ex = RM.buildReportModel(fixture({ changes: { improved: [{ label: 'Pickup', from: 10, to: 6, unit: 'h' }], declined: [] } }));
  assert.deepEqual(ex.changes.improved.map((x) => x.to), [6]);
  assert.equal(sectionHtml(RM.renderReport(RM.buildReportModel({}), {}), 'changes'), '', 'no comparison → no section');
});

test('DORA: deploy tag list (hotfix detected by name, newest first, tickets linked)', () => {
  const m = RM.buildReportModel(fixture({ dora: { deployments: 3, deploy_tags: [
    { name: 'release-deployed-1', repo: 'svc', ts: Date.parse('2026-08-01T10:00:00Z'), keys: ['ABC-1'] },
    { name: 'HF-payments-2', repo: 'svc', ts: Date.parse('2026-08-03T10:00:00Z') },
    { name: 'v1-hotfix', repo: 'web', ts: Date.parse('2026-08-02T10:00:00Z') },
  ] } }));
  assert.deepEqual(m.dora.deploy_tags.map((t) => [t.name, t.kind]), [['HF-payments-2', 'hotfix'], ['v1-hotfix', 'hotfix'], ['release-deployed-1', 'deploy']]);
  const html = sectionHtml(RM.renderReport(m, {}), 'dora');
  assert.match(html, /<summary>Deployment tags \(3, 2 hotfix\)<\/summary>/);
  assert.match(html, /<code>release-deployed-1<\/code>/);
  assert.match(html, /browse\/ABC-1/);
});

test('PR flow: merge row on the strip and a slow-PR table with ticket anchors', () => {
  const items = [
    { key: 'ABC-1', pr: 11, repo: 'svc', title: 'Speed up', person: 'Bob Sample', pickup_hours: 2, review_hours: 3, merge_hours: 30, size_lines: 120, comments: 4 },
    { key: 'ABC-22', pr: 12, person: 'Alice Example', pickup_hours: 1, review_hours: 1, merge_hours: 5 },
    { pr: 13, pickup_hours: 40, review_hours: 2 },
  ];
  const html = sectionHtml(RM.renderReport(RM.buildReportModel(fixture({ pr_flow: { prs: 3, items } })), {}), 'pr_flow');
  assert.match(html, /<div class="hc-l" title="Open → merged">/);
  const slow = html.slice(html.indexOf('Slowest pull requests'));
  assert.ok(slow.indexOf('#11') < slow.indexOf('#12'), 'sorted by open → merged');
  assert.match(slow, /<tr data-anchor="t:abc-1" data-anchor-label="ABC-1">/);
  assert.match(slow, /data-for="t:abc-1"[^>]*>\+<\/button>/);
});

test('comment anchors: every tile is section:metric, ticket rows are t:key, counts shown', () => {
  const m = RM.buildReportModel(rich());
  const html = RM.renderReport(m, {}, { comments: [{ anchor: 'dora:lead-time-for-changes', text: 'why so slow?' }, { anchor: 't:abc-1', text: 'see' }] });
  const tiles = html.match(/<div class="tile"[^>]*>/g);
  assert.ok(tiles.length > 10);
  for (const t of tiles) assert.match(t, /^<div class="tile" data-anchor="[a-z0-9_-]+:[a-z0-9_-]+" data-anchor-label="[^"]+">$/);
  assert.doesNotMatch(html, /<!--tp:cbtn-->|data-tile=/);
  assert.match(sectionHtml(html, 'dora'), /class="c-btn js-only mini has" data-for="dora:lead-time-for-changes" aria-label="Comment on Delivery \(DORA\) · Lead time for changes \(1\)"[^>]*>1</);
  assert.match(html, /<tr data-anchor="t:abc-1"/);
  assert.match(html, /data-goto="dora:lead-time-for-changes"/);
  assert.match(html, /href="#dora" data-goto="dora:lead-time-for-changes"/);
  // every anchor the page carries is one the comments API accepts
  const S = require('../lib/sanitize.js');
  for (const [, a] of html.replace(/<script[\s\S]*<\/script>/, '').matchAll(/data-anchor="([^"]+)"/g)) assert.ok(S.validAnchor(a), a);
});

test('masked: anchors scrubbed, free-text note shown, no real key in any attribute', () => {
  const m = RM.buildReportModel(rich({ mask: true }));
  const html = RM.renderReport(m, NARR);
  const anchors = [...html.matchAll(/data-(?:anchor|for|goto)="([^"]+)"/g)].map((x) => x[1]);
  assert.ok(anchors.some((a) => /^t:ticket-\d+$/.test(a)));
  for (const a of anchors) assert.doesNotMatch(a, /abc-|xyz-/);
  assert.match(html, /<li id="gr-\d+"><strong>mask\.free_text<\/strong> — Masking replaces known names/);
  assert.match(sectionHtml(html, 'blind_spots'), /Free text is not masked beyond known names/);
  assert.equal(sectionHtml(RM.renderReport(RM.buildReportModel(rich()), {}), 'blind_spots').includes('Free text is not masked'), false);
});

// ---------------------------------------------------------------- security
test('security: </script><script> in the title cannot open a second script', () => {
  const html = RM.renderReport(RM.buildReportModel(fixture({ meta: { title: '</script><script>alert(1)</script>' } })), {});
  assert.equal((html.match(/<script\b/gi) || []).length, 1);
  assert.equal((html.match(/<\/script>/gi) || []).length, 1);
  assert.match(html, /<title>&lt;\/script&gt;&lt;script&gt;alert\(1\)/);
});

test('security: jira_base must be a credential-free https origin', () => {
  for (const bad of ['javascript:alert(1)', 'https://u:p@x.example.com', 'https://u@x.example.com', 'data:text/html,x', 'HTTPS://x" onmouseover="1', '//x.example.com']) {
    const m = RM.buildReportModel(fixture({ jiraBase: bad }));
    assert.equal(m.meta.jira_base, null, bad);
    assert.doesNotMatch(RM.renderReport(m, {}), /\/browse\//, bad);
  }
});

test('security: a fresh nonce per render, matching the CSP and the only script tag', () => {
  const m = RM.buildReportModel(fixture());
  const nonces = new Set();
  for (let i = 0; i < 5; i++) {
    const html = RM.renderReport(m, {});
    const tags = html.match(/<script\b[^>]*>/g);
    assert.equal(tags.length, 1);
    const n = /nonce="([A-Za-z0-9]{16,})"/.exec(tags[0])[1];
    assert.ok(html.includes(`script-src 'nonce-${n}'`));
    assert.equal(html.split(n).length - 1, 2, 'nonce appears exactly in CSP + script tag');
    nonces.add(n);
  }
  assert.equal(nonces.size, 5);
});

test('security: <img src=x onerror=1> in EVERY string field reaches no unescaped sink', () => {
  const X = '<img src=x onerror=1>';
  // Every string leaf of the richest fixture → the payload (keys stay keys so links render).
  const poison = (v, k) => {
    if (typeof v === 'string') return /key$|^caused_by$|^key|by_key/.test(k || '') || k === 'jiraBase' || k === 'comments_endpoint' ? v : X;
    if (Array.isArray(v)) return v.map((x) => poison(x, k));
    if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([kk, x]) => [kk, poison(x, kk)]));
    return v;
  };
  const input = poison(rich({
    changes: { improved: [{ label: 'a', from: 1, to: 2, note: 'n', unit: 'u' }], declined: [] },
    dora: { deployments: 2, deploy_tags: [{ name: 'deployed-1', repo: 'r', ts: 1, keys: ['ABC-1'] }], failures: [{ key: 'ABC-1', kind: 'k', tag: 't' }] },
    pr_flow: { prs: 1, items: [{ key: 'ABC-1', pr: 'p', repo: 'r', title: 't', person: 'p', merge_hours: 3 }], by_person: [{ person: 'p' }], coverage_note: 'c' },
    glossary: [{ term: 't', definition: 'd' }],
    goals: [{ goal: 'g', target: 't', actual: 'a', note: 'n' }],
    report_kind: 'dev',
  }), '');
  input.report_kind = 'dev';
  const html = RM.renderReport(RM.buildReportModel(input), { summary: X, strengths: [X], goals: [X] }, { comments: [{ anchor: X, label: X, text: X, author: X }] });
  assert.doesNotMatch(html, /<img/i);
  const [, script] = /<script[^>]*>([\s\S]*)<\/script>/.exec(html);
  assert.doesNotMatch(html.replace(script, ''), /onerror=1(?!&)/, 'no live handler attribute outside escaped text');
  assert.doesNotMatch(script, /<img|<\/script/i, 'embedded JSON escapes "<"');
  assert.ok((html.match(/&lt;img src=x onerror=1&gt;/g) || []).length > 20, 'payload rendered as text');
  assert.equal((html.match(/<script\b/g) || []).length, 1);
});

test('security: ids and anchors pass through esc()', () => {
  const { sparkline, barChart } = require('../report/charts.js');
  const chart = barChart({ id: 'x"><img src=x>', title: 't', rows: [{ label: '"><b>', value: 1 }] });
  assert.doesNotMatch(chart, /<img|<b>/);
  assert.match(chart, /id="x&quot;&gt;&lt;img src=x&gt;"/);
  assert.doesNotMatch(sparkline({ values: [1, 2], label: '<b>x</b>' }), /<b>/);
  const html = RM.renderReport(RM.buildReportModel(fixture({ kpis: [{ id: 'a"b', label: '"><img src=x>', value: 1 }] })), {});
  assert.doesNotMatch(html, /<img/);
});

test('charts: a chart without its table fallback is refused', () => {
  const { requireTable } = require('../report/charts.js');
  assert.throws(() => requireTable('c', ''), /table fallback is required/);
  assert.match(requireTable('c', '<div><table></table></div>'), /^<details class="as-table">/);
});

test('charts: sparkline is aria-hidden with an sr-only trend line', () => {
  const { sparkline } = require('../report/charts.js');
  const s = sparkline({ values: [1, null, 3], labels: ['Q1', 'Q2', 'Q3'], label: 'Lead time', unit: 'd' });
  assert.match(s, /<svg class="spark-svg"[^>]*aria-hidden="true" focusable="false">/);
  assert.match(s, /<span class="sr-only">Lead time: Q1 1 d, Q2 not tracked, Q3 3 d<\/span>/);
  assert.equal(sparkline({ values: [1] }), '', 'one point is not a trend');
});

test('report.css: hchart stacks label over bar at 600px or below', () => {
  const css = require('fs').readFileSync(require('path').join(__dirname, '..', 'report', 'report.css'), 'utf8');
  const m = /@media \(max-width: 600px\) \{([\s\S]*?)\n\}/.exec(css);
  assert.ok(m, '600px block present');
  assert.match(m[1], /\.hc-row \{ grid-template-columns: minmax\(0, 1fr\)/);
  assert.match(m[1], /\.hc-l \{ text-align: start;/);
});

test('reportfeed shapes: phases_by_period rows, hotfix flag, PR size alias, top-level improved/declined', () => {
  const m = RM.buildReportModel(fixture({
    phases: { rows: [], by_period: [{ label: 'Q2', rows: [{ phase: 'dev', median_days: 4 }, { phase: 'review', median_days: 1 }] }] },
    dora: { deploy_tags: [{ name: 'release-7', ts: '2026-08-01T00:00:00Z', hotfix: true }] },
    pr_flow: { prs: 1, items: [{ id: 5, size: 300, merge_hours: 2 }] },
    improved: [{ label: 'Pickup', from: 10, to: 6, unit: 'h' }],
  }));
  assert.deepEqual([m.phases.by_period[0].dev, m.phases.by_period[0].review, m.phases.by_period[0].design], [4, 1, null]);
  assert.equal(m.dora.deploy_tags[0].kind, 'hotfix');
  assert.equal(m.pr_flow.items[0].size_lines, 300);
  assert.equal(m.pr_flow.items[0].pr, '5');
  assert.deepEqual(m.changes.improved.map((x) => x.label), ['Pickup']);
});
