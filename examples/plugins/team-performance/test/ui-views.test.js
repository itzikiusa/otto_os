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
    design: { n: 2, p50: 1, coverage: 10 },
    dev: { n: 20, p50: 3, coverage: 100 },
    review_pickup: { n: 0, p50: null, coverage: 0 },
    review_in_review: { n: 18, p50: 1, coverage: 90 },
    qa_wait: { n: 12, p50: 0.5, coverage: 60 },
    qa_rework: { n: 3, p50: 0.4, coverage: 15 },
    deploy: { n: 10, p50: 1, coverage: 50 },
    rework_in: { n: 6, p50: 0.5, coverage: 30 },
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

test('coverage is formatted with fmtPct whether it arrives as 0..1 or 0..100', () => {
  assert.match(TP.views.flow.phasesHtml({ phases: { team: { dev: 2 }, coverage: 0.4 } }), /on 40% of tickets/);
  assert.match(TP.views.flow.phasesHtml({ phases: { team: { dev: 2 }, coverage: 40 } }), /on 40% of tickets/);
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
