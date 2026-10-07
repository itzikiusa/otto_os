// UI views rendered as strings against a tiny stub DOM: honest states
// ("not tracked", DORA not available, PR empty states), capacity context
// beside every ratio, canonical-id guardrails, and every section state
// (loading / empty / error + Retry / loaded).
'use strict';
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

// ---- stub DOM ---------------------------------------------------------------
class El {
  constructor(tag) {
    this.tag = tag;
    this.children = [];
    this._html = '';
    this.attrs = {};
    this.style = {};
    this.dataset = {};
    this.className = '';
  }
  set innerHTML(v) {
    this._html = String(v);
    this.children = [];
    this.body = null;
    this.retryBtn = null;
    if (/class="body"/.test(this._html)) {
      this.body = new El('div');
      this.children.push(this.body);
    }
  }
  get innerHTML() {
    return this._html;
  }
  appendChild(c) {
    this.children.push(c);
    return c;
  }
  insertAdjacentHTML(_pos, h) {
    this._html += h;
  }
  querySelector(sel) {
    if (sel === '.body') return this.body || null;
    if (sel === '.retry' && /class="[^"]*\bretry\b/.test(this._html)) return this.retryBtn || (this.retryBtn = { onclick: null });
    if (sel.startsWith('#') && this.html().includes(`id="${sel.slice(1)}"`)) return (this.stubs = this.stubs || {})[sel] || (this.stubs[sel] = { onclick: null, onchange: null });
    return null;
  }
  querySelectorAll() {
    return [];
  }
  setAttribute(k, v) {
    this.attrs[k] = v;
  }
  removeAttribute(k) {
    delete this.attrs[k];
  }
  addEventListener() {}
  html() {
    return this._html + this.children.map((c) => c.html()).join('');
  }
}

let narrowScreen = false;
let fetchImpl = async () => ({ ok: true, json: async () => ({}) });
global.window = global;
global.matchMedia = () => ({ matches: narrowScreen });
global.document = {
  addEventListener() {},
  createElement: (t) => new El(t),
  documentElement: { clientWidth: 1200, clientHeight: 800 },
  querySelector: () => null,
  getElementById: () => null,
  body: new El('body'),
};
global.fetch = (...a) => fetchImpl(...a);
global.addEventListener = () => {};

const UI = path.join(__dirname, '..', 'ui');
for (const f of ['components.js', 'views/overview.js', 'views/flow.js', 'views/quality.js', 'views/investment.js', 'views/people.js', 'views/person.js', 'views/estimates.js', 'views/reports.js']) {
  vm.runInThisContext(fs.readFileSync(path.join(UI, f), 'utf8'), { filename: f });
}
const TP = global.TP;

const tick = async (n = 6) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setImmediate(r));
};
const respond = (body, status = 200) => async () => ({ ok: status < 300, status, json: async () => body });
const app = {
  account: 'acct',
  config: { deploy_tag_patterns: ['deployed', 'hf', 'hotfix'] },
  scan: null,
  people: {},
  tab: 'overview',
  goTab() {},
  periodLabel: () => 'Last 3 months',
  scopeQ: () => 'account=acct',
};
async function renderView(name, o, extra = {}) {
  const host = new El('div');
  TP.views[name].render(host, { o, app: { ...app, ...extra }, host });
  await tick();
  return host.html();
}

// ---- fixture payload (generic: ABC-123 keys, Person A/B) --------------------
const fixture = () => ({
  completed: 20,
  open: 3,
  scope: { median_cycle_days: 4, fix_rate: 0.1, weighted_throughput_wk: 6 },
  capacity: {
    capacity_days: 110,
    business_days: 120,
    time_off_days: 10,
    people: {
      a: { name: 'Person A', business_days: 20, time_off_days: 2, capacity_days: 18 },
      b: { name: 'Person B', business_days: 20, time_off_days: 12, capacity_days: 8 },
    },
  },
  dora: {
    deploy_frequency: { status: 'not_available', value: null, total: 0, patterns: ['deployed', 'hf', 'hotfix'], repos: ['repo-alpha'], branch: 'origin/develop', band: 'low' },
    lead_time: { value: 3, p50: 3, n: 7, band: 'high' },
    change_failure_rate: { value: 0.25, failed: 1, total: 4, band: 'medium' },
    mttr: { value: 5, p50: 5, unit: 'hours', n: 1, band: 'elite' },
  },
  guardrails: [{ id: 'low_estimate_coverage:estimateAccuracy', code: 'low_estimate_coverage', metric: 'estimateAccuracy', level: 'warn', msg: 'Only 40% of delivered tickets have an estimate.' }],
  guardrail_badges: { throughputPerWeek: { severity: 'danger', codes: ['low_capacity'], reasons: ['Available on 8 of 20 working days.'] } },
  phases: {
    design: { n: 2, p50: 1, coverage: 0.1 },
    dev: { n: 20, p50: 3, coverage: 1 },
    review_pickup: { n: 0, p50: null, coverage: 0 },
    review_in_review: { n: 18, p50: 1, coverage: 0.9 },
    qa_wait: { n: 12, p50: 0.5, coverage: 0.6 },
    qa_rework: { n: 3, p50: 0.4, coverage: 0.15 },
    deploy: { n: 10, p50: 1, coverage: 0.5 },
    rework_in: { n: 6, p50: 0.5, coverage: 0.3 },
  },
  pr_flow: { total: 0 },
  flow: {},
  investment: { by_type: { Story: { days: 30, share: 0.6 }, Bug: { days: 20, share: 0.4 } }, by_epic: { 'ABC-1': { days: 30, share: 0.6 }, 'no epic': { days: 20, share: 0.4 } }, epics: { 'ABC-1': { summary: 'Checkout revamp' } } },
  assignees: [
    { assignee_id: 'a', assignee_name: 'Person A', completed: 12, weighted_done: 9, capacity_days: 18, time_off_days: 2, weighted_per_capacity_day: 0.5, pace_vs_est: 1.2 },
    { assignee_id: 'b', assignee_name: 'Person B', completed: 3, weighted_done: 4, capacity_days: 8, time_off_days: 12, weighted_per_capacity_day: 0.5 },
  ],
  subtasks: [{ key: 'ABC-9', parent_key: 'ABC-2', summary: 'Wire the API', assignee_id: 'b', assignee_name: 'Person B', credited_to: 'b', rollup: false, dev_days: 2 }],
});

// ---- guardrails --------------------------------------------------------------
test('guardFor looks guardrails up by canonical metric id, never by message text', () => {
  const o = fixture();
  assert.equal(TP.guardFor(o, ['throughputPerWeek']).level, 'bad');
  assert.equal(TP.guardFor(o, ['estimateAccuracy']).level, 'warn');
  // the message mentions "estimate" but the metric id does not match
  assert.equal(TP.guardFor({ guardrails: [{ id: 'x:prPickup', level: 'warn', msg: 'estimate coverage low' }] }, ['estimateAccuracy']), null);
  // id "code:metric" works when `metric` is absent
  assert.ok(TP.guardFor({ guardrails: [{ id: 'low_n:leadTime', level: 'bad', msg: 'n=2' }] }, ['leadTime']));
});

test('overview shows the guardrail banner and per-tile guardrail badges', async () => {
  const html = await renderView('overview', fixture());
  assert.match(html, /input check.* failing/);
  assert.match(html, /weak inputs/); // throughputPerWeek badge on Delivered scope
  assert.match(html, /capacity person-days \(120 working − 10 off\)/);
  assert.match(html, /never a productivity score/);
});

// ---- DORA ---------------------------------------------------------------------
test('DORA deploy frequency "Not available" carries patterns/repos/branch + Settings action and no band', () => {
  const html = TP.views.overview.doraTiles(fixture(), app);
  const deployTile = html.split('<div class="tile">')[1];
  assert.match(deployTile, /Not available/);
  assert.match(deployTile, /deployed, hf, hotfix/);
  assert.match(deployTile, /repo-alpha/);
  assert.match(deployTile, /origin\/develop/);
  assert.match(deployTile, /data-goto="settings"/);
  assert.doesNotMatch(deployTile, /class="badge[^"]*"[^>]*>.*low/);
  assert.match(html, /1 of 4 deploys failed/);
  assert.match(html, /median of 1 incident\b/);
  assert.match(html, /25%/);
});

// ---- flow ---------------------------------------------------------------------
test('flow phases: thin phases read "not tracked" (hatched) with design / QA coverage notes', () => {
  const html = TP.views.flow.phasesHtml(fixture());
  assert.match(html, /not tracked/);
  assert.match(html, /hatch/);
  assert.match(html, /Design: tracked on 2 of 20 tickets/);
  assert.match(html, /Tickets with any QA rework: 3 of 20/);
  assert.match(html, /Review \(from Jira statuses\)/);
  const withPrs = TP.views.flow.phasesHtml({ ...fixture(), pr_flow: { total: 12 } });
  assert.doesNotMatch(withPrs, /from Jira statuses/);
});

test('coverage is a 0..1 share; a value above 1 reads ">100%" with a warning, never rescaled', () => {
  assert.match(TP.views.flow.phasesHtml({ phases: { team: { dev: 2 }, coverage: 0.4 } }), /on 40% of tickets/);
  const over = TP.views.flow.phasesHtml({ phases: { team: { dev: 2 }, coverage: 40 } });
  assert.match(over, /on &gt;100% of tickets/);
  assert.match(over, /overlapping inputs/);
  assert.doesNotMatch(over, /on 40% of tickets/);
});

test('PR section: unavailable → reason + Settings; running → paced progress; idle → Scan now', () => {
  const un = TP.views.flow.prEmptyHtml({ state: 'unavailable', available: false, error: 'no daemon API token' }, app);
  assert.match(un, /Pull-request data is unavailable/);
  assert.match(un, /no daemon API token/);
  assert.match(un, /data-pr-act="settings"/);
  const run = TP.views.flow.prEmptyHtml({ state: 'running', next_call_in_ms: 1500, pacer: { calls: 4, last_backoff_ms: 4000 } }, app);
  assert.match(run, /≥2 s per call/);
  assert.match(run, /next call in 2s/);
  assert.match(run, /backing off/);
  const idle = TP.views.flow.prEmptyHtml({ state: 'idle', at: null }, app);
  assert.match(idle, /data-pr-act="scan"/);
  const unreg = TP.views.flow.prEmptyHtml({ state: 'done', at: Date.now(), unregistered: ['/repos/x'] }, app);
  assert.match(unreg, /not registered in Otto/);
});

test('PR section states: error + Retry, then loaded empty state', async () => {
  fetchImpl = respond({ error: 'boom' }, 500);
  const host = new El('div');
  TP.views.flow.render(host, { o: fixture(), app, host });
  assert.match(host.html(), /Loading…/); // loading state before the fetch settles
  await tick();
  const prSection = host.children[0].children[1].children[0];
  assert.match(prSection.html(), /Couldn’t load pull requests: boom/);
  assert.match(prSection.html(), />Retry</);
  fetchImpl = respond({ state: 'unavailable', available: false, error: 'no token' });
  prSection.body.querySelector('.retry').onclick();
  await tick();
  assert.match(prSection.html(), /Pull-request data is unavailable/);
});

// ---- investment ----------------------------------------------------------------
test('investment: epic rows carry summary + Jira key; "no epic" is explained', async () => {
  const html = await renderView('investment', fixture());
  assert.match(html, /ABC-1/);
  assert.match(html, /Checkout revamp/);
  assert.match(html, /No epic/);
  assert.match(html, /Not linked to an epic in Jira/);
});

// ---- people / person -----------------------------------------------------------
test('people rows show capacity (business − off = capacity) beside each rate, plus a row guardrail', () => {
  const o = fixture();
  const rows = TP.views.people.rowsHtml(o, o.assignees);
  const a = rows[0].cells.join(' ');
  assert.match(a, /18 <span class="dim small">= 20 − 2 off/);
  assert.match(a, /0\.5 <span class="dim small">over 18 d/);
  const b = rows[1].cells.join(' ');
  assert.match(b, /check inputs|weak inputs/); // 8 of 20 days + only 3 tickets
  assert.match(TP.views.people.rowGuard(o, o.assignees[1]).msg, /Available 8 of 20 working days/);
});

test('people view: not-a-productivity-score note, capacity column, credited sub-tasks', async () => {
  const html = await renderView('people', fixture());
  assert.match(html, /Not a productivity score/);
  const sub = TP.views.people.subtasksHtml(fixture());
  assert.match(sub, /ABC-9/);
  assert.match(sub, /Person B/);
  assert.match(sub, /counted separately/);
  assert.match(TP.views.people.subtasksHtml({ subtasks: [] }), /No sub-tasks carried real work/);
});

test('person phase cells: null days read "not tracked", never 0', () => {
  const cell = TP.views.person.phaseCell;
  assert.match(cell({ phases: { design: { days: null } } }, 'design'), /not tracked/);
  assert.match(cell({ phases: { design: null } }, 'design'), /not tracked/);
  assert.equal(cell({ phases: { dev: { days: 2 } } }, 'dev'), '2d');
  assert.equal(cell({ phases: { review: { total: 1.5 } } }, 'review'), '1.5d');
});

// ---- quality --------------------------------------------------------------------
test('quality tiles show CFR as "N of M deploys"', () => {
  const html = TP.views.quality.tiles(fixture());
  assert.match(html, /1 of 4 deploys/);
});

// ---- reports ---------------------------------------------------------------------
test('reports: masking badge is consistent and comments render', () => {
  assert.match(TP.views.reports.maskBadge({ masked: false }), /badge warning.*names visible/);
  assert.match(TP.views.reports.maskBadge({ masked: true }), /badge info.*names masked/);
  assert.match(TP.views.reports.commentsHtml([]), /No comments yet/);
  assert.match(TP.views.reports.commentsHtml([{ author: 'lead', text: 'Check <b>' , at: 0 }]), /Check &lt;b&gt;/);
});

test('reports list: empty, error + Retry, loaded with Regenerate/Delete', async () => {
  fetchImpl = async (url) => ({ ok: true, json: async () => (String(url).includes('active') ? { active: [] } : { reports: [] }) });
  assert.match(await renderView('reports', null, { tab: 'reports' }), /No reports yet/);
  fetchImpl = respond({ error: 'offline' }, 503);
  assert.match(await renderView('reports', null, { tab: 'reports' }), /Couldn’t load reports: offline.*Retry/s);
  fetchImpl = async (url) => ({
    ok: true,
    json: async () => (String(url).includes('active') ? { active: [] } : { reports: [{ id: 'r1', report_scope: 'team', label: '2026 Q3', masked: false, created_at: 0 }] }),
  });
  const html = await renderView('reports', null, { tab: 'reports' });
  assert.match(html, /Regenerate/);
  assert.match(html, /Delete/);
  assert.match(html, /names visible/);
});

// ---- narrow screens -----------------------------------------------------------------
test('below 600px charts open "Show as table" by default', () => {
  narrowScreen = true;
  try {
    assert.match(TP.views.flow.phasesHtml(fixture()), /<details open><summary>Show as table/);
  } finally {
    narrowScreen = false;
  }
  assert.doesNotMatch(TP.views.flow.phasesHtml(fixture()), /<details open>/);
});

// ---- P8: shares, guardrails, privacy, deep links, a11y ------------------------
const P8_SRC = (f) => fs.readFileSync(path.join(UI, f), 'utf8');

test('(1) no 0..100 guessing anywhere in ui/: share01 and "> 1 ? v / 100" are gone', () => {
  for (const f of ['components.js', 'views/overview.js', 'views/flow.js', 'views/quality.js', 'views/investment.js', 'views/people.js', 'views/person.js', 'views/estimates.js']) {
    const src = P8_SRC(f);
    assert.doesNotMatch(src, /share01/, f);
    assert.doesNotMatch(src, />\s*1\s*\?\s*[\w.]+\s*\/\s*100/, f);
  }
  assert.equal(TP.fmtShare(0.25), '25%');
  assert.equal(TP.fmtShare(1), '100%');
  assert.equal(TP.fmtShare(25), '>100%');
  assert.equal(TP.fmtShare(null), '—');
  assert.equal(TP.overGuard(0.5), null);
  assert.equal(TP.overGuard(1.4, 'Rework rate').level, 'bad');
});

test('(1)+(2) a share above 1 renders ">100%" with a bad guardrail, dimmed + reason inline', async () => {
  const o = fixture();
  o.dora.change_failure_rate = { value: 3, failed: 3, total: 1 };
  o.rework = { rate: 1.5, charged: [] };
  const dora = TP.views.overview.doraTiles(o, app);
  assert.match(dora, /&gt;100%/);
  assert.match(dora, /class="tile weak"/);
  assert.match(dora, /guard-reason/);
  const q = TP.views.quality.tiles(o);
  assert.match(q, /Rework rate[\s\S]*&gt;100%/);
  assert.match(q, /weak inputs/);
});

test('(2) guardFor matches canonical ids, tiles[] and envelope weak_reasons; bad → hatched tile with reason', () => {
  const o = { guardrails: [{ id: 'pr_no_data', code: 'no_pr_data', metric: '*', tiles: ['pr_pickup', 'pr_review'], level: 'bad', msg: 'No pull requests fetched.' }] };
  assert.equal(TP.guardFor(o, ['prPickup']).level, 'bad');
  assert.equal(TP.guardFor(o, ['dora_cfr']), null);
  assert.equal(TP.guardFor({}, ['x'], { weak_reasons: ['low_n'] }).level, 'warn');
  const html = TP.tile({ title: 'Pickup', valueHtml: '1d', guard: TP.guardFor(o, ['prPickup']) });
  assert.match(html, /tile weak/);
  assert.match(html, /<p class="guard-reason">.*No pull requests fetched/s);
  assert.match(html, /weak inputs/);
  const warn = TP.tile({ title: 'X', valueHtml: '1', guard: { level: 'warn', msg: 'm' } });
  assert.doesNotMatch(warn, /tile weak|guard-reason/);
  assert.match(warn, /check inputs/);
});

test('(2) Investment, PR and Quality sections carry guardrail badges', async () => {
  const o = fixture();
  o.guardrails.push({ id: 'capacity_investment', metric: 'investmentMix', level: 'bad', msg: 'No git evidence on most tickets.' });
  const inv = await renderView('investment', o);
  assert.match(inv, /weak inputs/);
  assert.match(inv, /No git evidence on most tickets/);
  const pr = TP.views.flow.prHtml({ ...o, guardrails: [{ id: 'pr_pickup', metric: 'prPickup', level: 'bad', msg: 'Only 1 PR.' }], pr_flow: { total: 1, pickup_days: { value: 1, n: 1 } } });
  assert.match(pr, /stage weak/);
  assert.match(pr, /Only 1 PR/);
});

test('(3) report masking: on by default for team/combined, off only for one person from their page', () => {
  const R = TP.views.reports;
  assert.equal(R.maskDefault('team', false), true);
  assert.equal(R.maskDefault('combined', true), true);
  assert.equal(R.maskDefault('dev', false), true);
  assert.equal(R.maskDefault('dev', true), false);
  assert.equal(R.namesLine(false), 'Names: visible to anyone you share with');
  assert.match(R.namesLine(true), /masked/);
  const src = P8_SRC('views/reports.js');
  assert.match(src, /id="nr-names" role="status" aria-live="polite"/);
  assert.match(P8_SRC('views/person.js'), /fromPerson: true/);
});

test('(4) calibration rows carry an info() each; inflation > 1.1 makes adjusted pace the headline', () => {
  const html = TP.views.estimates.calibrationHtml({ calibration: { corrections: 4, factor: 1.1, ruler_version: 'r3' } }, { pace: 1.2, est_inflation: 1.3, pace_adjusted: 1.56, pace_ref: 1.4 });
  assert.ok((html.match(/class="info-btn"/g) || []).length >= 6);
  assert.match(html, /class="headline"><span class="value">×1\.56/);
  assert.ok(html.indexOf('Pace, inflation-adjusted') < html.indexOf('>Pace <'), 'adjusted row comes first');
  const flat = TP.views.estimates.calibrationHtml(null, { pace: 1.2, est_inflation: 1.05, pace_adjusted: 1.26 });
  assert.doesNotMatch(flat, /class="headline"/);
  const ov = TP.views.overview.flowTiles({ ...fixture(), est_basis: { pace: 1.2, est_inflation: 1.3, pace_adjusted: 1.56, pace_ref: 1.4 } });
  assert.match(ov, /Pace vs estimate \(inflation-adjusted\)[\s\S]*×1\.56/);
});

test('(5) rework empty state names the sources that ran, or says "not checked yet", with Scan/Settings', () => {
  const ran = TP.views.quality.chargedHtml({ rework: { charged: [], sources: { git_blame: { repos: 4 }, jira_links: { tickets: 120 } } } });
  assert.match(ran, /git blame scanned 4 repos, Jira links checked 120 tickets — none found/);
  assert.match(ran, /data-empty-act="scan"/);
  assert.match(ran, /data-empty-act="settings"/);
  assert.match(TP.views.quality.chargedHtml({}), /not checked yet/);
});

test('(5) hygiene flags get human labels and open a drawer of tickets', () => {
  const html = TP.views.quality.flagsHtml({ flags: { late_merge: 3, no_code: 1 } });
  assert.match(html, /Merged after done × 3/);
  assert.match(html, /No code found × 1/);
  assert.doesNotMatch(html, /late merge|late_merge ×/);
  assert.match(html, /<button type="button" class="flag" data-drill="d\d+"/);
});

test('(6) People has a sub-task count column; person page section is "Sub-tasks with real work"', () => {
  const o = fixture();
  assert.ok(TP.views.people.COLS.some((c) => c.key === 'subs'));
  assert.equal(TP.views.people.subCount(o, 'b'), 1);
  assert.equal(TP.views.people.subCount(o, 'a'), 0);
  assert.equal(TP.views.people.subCount({}, 'a'), null);
  const rowB = TP.views.people.rowsHtml(o, o.assignees)[1].cells.join(' ');
  assert.match(rowB, /1 sub-tasks with real work/);
  assert.match(P8_SRC('views/person.js'), /title: 'Sub-tasks with real work'/);
  const sub = TP.views.people.subtasksHtml(o);
  assert.match(sub, /badge info/);
});

test('(7) deep links: route parses and builds tab/person/period; tile values with tickets are drawer buttons', () => {
  assert.deepStrictEqual(TP.route.parse('#tab=people&person=a&period=3'), { tab: 'people', person: 'a', period: '3' });
  assert.deepStrictEqual(TP.route.parse('#period=bogus&since=x'), {});
  assert.equal(TP.route.build({ tab: 'flow', person: null, period: 'custom', since: '2026-01-01' }), '#tab=flow&period=custom&since=2026-01-01');
  const html = TP.tile({ title: 'Escapes', valueHtml: '10%', drill: { tickets: [{ key: 'ABC-1', summary: 'S' }] } });
  assert.match(html, /<button type="button" class="tile-value value" data-drill="d\d+" aria-haspopup="dialog"/);
  assert.match(TP.drawerHtml({ tickets: [{ key: 'ABC-1', summary: 'Login', value: 2 }] }), /ABC-1[\s\S]*Login/);
  assert.match(TP.drawerHtml({ tickets: [] }), /did not return the tickets/);
  const app = P8_SRC('views/app.js');
  assert.match(app, /TP\.route\.parse\(location\.hash\)/);
  assert.match(app, /hashchange/);
});

test('(8) second-tier signal tiles render; capacity-normalised deploys show capacity context', () => {
  const o = fixture();
  o.flow = {
    flow_efficiency: { value: 0.32, n: 18 },
    focus_share: { value: 0.7, n: 40, focused_days: 28 },
    escape_rate: { value: 0.1, n: 20, escaped: 2, items: [{ key: 'ABC-3', bugs: ['ABC-9'] }] },
    sprint_planning: { value: { accuracy: 0.8, scope_creep: 0.2 }, n: 3 },
  };
  o.pr_flow = { total: 9, review_load: { n: 20, top_share: 0.6, reviewers_n: 3, reviewers: [] }, by_person: { x: { authored: 2, reviewed_given: 3 }, y: { authored: 1, reviewed_given: 0 } } };
  o.dora.hotfix_rate = { value: 0.25, hotfixes: 1, total: 4 };
  o.dora.batch_size = { value: 3, n: 4 };
  o.dora.time_to_detect = { value: 5, n: 2 };
  o.dora.deploys_per_capacity_day = { value: 0.04, n: 4 };
  const html = TP.views.overview.signalTiles(o);
  for (const t of ['Flow efficiency', 'Focus', 'Escape rate', 'Sprint plan accuracy', 'Review load spread', 'People in PRs', 'Hotfix rate', 'Batch size', 'Time to detect', 'Time to restore', 'Deploys per capacity day']) assert.match(html, new RegExp(t), t);
  assert.match(html, /32%/);
  assert.match(html, /scope creep 20%/);
  assert.match(html, /2 authored · 1 reviewed/);
  assert.match(html, /over 110 capacity person-days \(120 working − 10 off\)/);
  assert.match(html, /data-drill=/); // escape rate items open a drawer
  assert.match(TP.views.overview.signalTiles({}), /not available yet/);
});

test('(8) per-person rates carry capacity context (people rows + person PR tile)', () => {
  const o = fixture();
  const a = TP.views.people.rowsHtml(o, o.assignees)[0].cells.join(' ');
  assert.match(a, /over 18 d/);
  assert.match(a, /independent of capacity/);
  assert.match(P8_SRC('views/person.js'), /reviews per capacity day/);
});

test('(10) a11y: sort glyphs aria-hidden, scrollable table region, sparkline sr text, NT + unestimated markers', () => {
  const t = TP.table({ caption: 'Cap', cols: [{ key: 'a', label: 'A', sort: true }], rows: [['x']], sortKey: 'a', sortDir: 'asc' });
  assert.match(t, /<span aria-hidden="true"> ▲<\/span>/);
  assert.match(t, /<div class="table-wrap" tabindex="0" role="region" aria-label="Cap">/);
  const sp = TP.sparkline([1, 2, 3], 'trend');
  assert.match(sp, /<svg class="spark"[^>]*aria-hidden="true"/);
  assert.match(sp, /<span class="sr-only">trend: 3 points, from 1 to 3<\/span>/);
  assert.throws(() => TP.chartBlock({ svg: '<svg></svg>' }), /tableHtml is required/);
  assert.match(TP.NT, /<abbr class="nt" title="Not tracked/);
  assert.match(TP.unestimated(), /badge warning.*unestimated/);
  const flowTbl = TP.views.flow.phasesHtml(fixture());
  assert.match(flowTbl, /<abbr class="nt"/);
});

test('(10) a modal makes main + toolbar inert and restores them on close', () => {
  const inert = {};
  const mk = (n) => ({ setAttribute: (k) => (inert[n] = k === 'inert'), removeAttribute: () => (inert[n] = false) });
  const els = { main: mk('main'), '.toolbar': mk('toolbar') };
  const dlg = new El('div');
  let dialogFocused = false;
  dlg.focus = () => { dialogFocused = true; };
  dlg.querySelectorAll = () => [];
  dlg.querySelector = () => null;
  const back = new El('div');
  Object.defineProperty(back, 'firstElementChild', { get: () => dlg });
  back.remove = () => {};
  const prevQS = document.querySelector;
  const prevCE = document.createElement;
  const prevRm = document.removeEventListener;
  document.querySelector = (s) => els[s] || null;
  document.createElement = () => back;
  document.removeEventListener = () => {};
  try {
    const m = TP.modal({ title: 'x', body: 'y' });
    assert.equal(dialogFocused, true, 'an actionless dialog receives keyboard focus');
    assert.deepStrictEqual(inert, { main: true, toolbar: true });
    m.close();
    assert.deepStrictEqual(inert, { main: false, toolbar: false });
  } finally {
    document.querySelector = prevQS;
    document.createElement = prevCE;
    document.removeEventListener = prevRm;
  }
});

test('(9) applyTheme input: every host variable is copied and the missing surfaces are derived', () => {
  const v = new Map(TP.themeVars({ '--bg': 'X', text: 'Y', '--surface-2': 'Z', '--cat-1': 'C', bad: 3, 'a;b': 'q' }));
  assert.equal(v.get('--bg'), 'X');
  assert.equal(v.get('--text'), 'Y');
  assert.equal(v.get('--cat-1'), 'C');
  assert.ok(!v.has('--bad') && !v.has('--a;b'));
  for (const k of ['--surface-3', '--hover', '--border-strong', '--bg-sidebar']) assert.match(v.get(k), /color-mix/, k);
  const app = P8_SRC('views/app.js');
  assert.match(app, /TP\.themeVars\(theme\)/);
  assert.doesNotMatch(app, /k\.startsWith\('--'\)\) root\.style/);
});

test('(9) layout: no inline layout styles in reports/settings; rv-* and form-actions classes exist', () => {
  for (const f of ['views/reports.js', 'views/settings.js']) assert.doesNotMatch(P8_SRC(f), /style="[^"]*(display|flex|margin|grid)/, f);
  assert.doesNotMatch(P8_SRC('views/reports.js'), /\.style\.(display|gridTemplateRows|minBlockSize)/);
  const css = P8_SRC('app.css');
  for (const c of ['.rv-head', '.rv-layout', '.rv-frame', '.form-actions']) assert.ok(css.includes(c + ' {') || css.includes(c + ',') || css.includes(c + ' '), c);
  assert.match(css, /@media \(max-width: 600px\) \{\s*\.rv-layout \{\s*grid-template-columns: minmax\(0, 1fr\)/);
});

test('quality review: weak tiles keep full reasons in a collapsed native disclosure', () => {
  const reason = 'A long shared input limitation. '.repeat(12);
  const html = TP.tile({ title: 'Lead time', valueHtml: '11d', guard: { level: 'bad', msg: reason } });
  assert.match(html, /<details class="guard-details"><summary>Input limitations<\/summary>/);
  assert.doesNotMatch(html, /<details[^>]*\sopen(?:\s|>)/);
  assert.ok(html.includes(reason), 'all explanatory text remains reachable');
  assert.match(html, /weak inputs/, 'the warning remains visible while reasons are collapsed');
});
