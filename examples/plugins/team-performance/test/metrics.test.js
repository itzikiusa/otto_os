'use strict';
const test = require('node:test');
const assert = require('node:assert');
const M = require('../lib/metrics.js');

const DAY = 86400000;
const t0 = Date.UTC(2026, 0, 5, 9); // Monday

function scopeOf(records, people = {}) {
  return {
    records, estimates: { 'ABC-1': { days: 2 }, 'ABC-2': { days: 1 } }, people, flat_people: { u1: { name: 'Person One' } },
    canonical: (id) => id, config: { workweek: [1, 2, 3, 4, 5], qa_work_min_commit_days: 2, timezone: 'UTC' },
  };
}

test('weekendOf converts a workweek into weekend days', () => {
  assert.deepStrictEqual(M.weekendOf([1, 2, 3, 4, 5]), [0, 6]);
  assert.deepStrictEqual(M.weekendOf([0, 1, 2, 3, 4]), [5, 6]);
});

test('offDaySet expands inclusive time-off ranges', () => {
  const s = M.offDaySet({ time_off: [{ from: '2026-01-05', to: '2026-01-07' }] });
  assert.deepStrictEqual([...s], ['2026-01-05', '2026-01-06', '2026-01-07']);
});

test('computeMetrics: capacity uses time off, design not tracked stays null, rework excluded from scope', () => {
  const recs = [
    { key: 'ABC-1', type: 'Story', assignee_id: 'u1', status_category: 'done', status: 'Done', created: t0 - DAY, done_at: t0 + 3 * DAY, eff_done_at: t0 + 3 * DAY, first_active_at: t0, intervals: [{ status: 'In Progress', from: t0, to: t0 + 2 * DAY }], commit_ts: [t0 + DAY] },
    { key: 'ABC-2', type: 'Bug', assignee_id: 'u1', status_category: 'done', status: 'Done', created: t0 + 4 * DAY, done_at: t0 + 6 * DAY, eff_done_at: t0 + 6 * DAY, first_active_at: t0 + 4 * DAY, intervals: [], scope_excluded: true, rework_of: 'ABC-1' },
  ];
  const withPh = M.attachPhases(recs, { prMap: new Map(), tags: [], config: scopeOf([]).config, people: {}, canonical: (x) => x });
  assert.strictEqual(withPh[0].phases.design.days, null, 'no design evidence → not tracked');
  const scope = scopeOf(withPh, { u1: { time_off: [{ from: '2026-01-12', to: '2026-01-16' }] } });
  const m = M.computeMetrics(scope, { since: t0, until: t0 + 14 * DAY }, { tags: [], prs: null });
  assert.strictEqual(m.capacity.people.u1.time_off_days, 5);
  assert.strictEqual(m.capacity.people.u1.capacity_days, 5);
  assert.ok(m.guardrails.some((g) => g.code === 'no_deploy_tags'));
  assert.strictEqual(m.flow.throughputPerWeek.value.count, 1, 'rework ticket is not delivered scope');
});

test('attachPhases: reachability deploy, phaseTags keep repo, activity from raw corpus, links lookup', () => {
  const recs = [
    {
      key: 'ABC-1', type: 'Story', assignee_id: 'u1', status_category: 'done', done_at: t0 + 5 * DAY, eff_done_at: t0 + 5 * DAY,
      intervals: [{ status: 'In Progress', from: t0 + 2 * DAY, to: t0 + 3 * DAY }], commit_ts: [],
      done_git_at: t0 + 3 * DAY, deployed_at: t0 + 4 * DAY, deployed_tag: 'v1-deployed', git_change: { repos: [{ name: 'svc' }] },
    },
    { key: 'ABC-2', type: 'Spike', status_category: 'done', first_active_at: t0, done_at: t0 + DAY },
  ];
  const corpusIndex = new Map([['ABC-1', { changelog: { histories: [{ created: new Date(t0 + DAY).toISOString(), author: { accountId: 'u1' } }] }, links: [{ key: 'ABC-2', type: 'Spike', summary: 'options' }] }]]);
  const out = M.attachPhases(recs, { prMap: new Map(), tags: [{ name: 'x-deployed', ts: t0 + 9 * DAY, repo: 'svc', sha: 'abc' }], config: scopeOf([]).config, people: {}, canonical: (x) => x, corpusIndex });
  const ph = out[0].phases;
  assert.deepStrictEqual(ph.deploy, { days: 1, tag: 'v1-deployed', source: 'reachability' });
  assert.deepStrictEqual(ph.design.evidence, ['linked']);
  assert.strictEqual(ph.design.days, 1, 'linked spike timing clipped before first dev');
  assert.strictEqual(ph.rework.reason, 'no_git_blame');
  assert.strictEqual(ph.mismatch, false);
  assert.deepStrictEqual(M.activityOfRaw(corpusIndex.get('ABC-1')), [{ at: t0 + DAY, author_id: 'u1' }]);
});

test('attachPhases: lead override clips to the effective window, labels dev and flags mismatch', () => {
  const recs = [{
    key: 'ABC-1', type: 'Story', assignee_id: 'u1', status_category: 'done', manual_days: 10,
    eff_start_at: t0 + DAY, eff_done_at: t0 + 3 * DAY, done_at: t0 + 3 * DAY,
    intervals: [{ status: 'In Progress', from: t0, to: t0 + 4 * DAY }],
  }];
  const ph = M.attachPhases(recs, { prMap: new Map(), tags: [], config: scopeOf([]).config, people: {}, canonical: (x) => x })[0].phases;
  assert.strictEqual(ph.dev.days, 2);
  assert.strictEqual(ph.dev.label, 'lead-corrected');
  assert.strictEqual(ph.mismatch, true);
  assert.deepStrictEqual(ph.mismatch_detail, { phase_sum: 2, corrected_actual: 10 });
});

test('attachPhases: design sub-tasks are excluded from child_dev_days', () => {
  const recs = [
    { key: 'ABC-1', type: 'Story', child_dev_days: 5, intervals: [] },
    { key: 'ABC-2', subtask: true, parent_key: 'ABC-1', type: 'Sub-task', summary: 'Design the schema', dev_days: 2 },
    { key: 'ABC-3', subtask: true, parent_key: 'ABC-1', type: 'Sub-task', summary: 'Implement', dev_days: 3 },
  ];
  const out = M.attachPhases(recs, { prMap: new Map(), tags: [], config: scopeOf([]).config });
  assert.strictEqual(out[0].child_dev_days, 3);
  assert.strictEqual(out[0].child_dev_days_design_excluded, 2);
});

test('phase guardrails: canonical phase_<name>_weak when n<5 or coverage<0.5; banner id is the code', () => {
  const summary = {
    dev: { n: 10, p50: 1, p75: 2, coverage: 1 },
    design: { n: 6, p50: 1, p75: 1, coverage: 0.3 },
    deploy: { n: 2, p50: 1, p75: 1, coverage: 0.9 },
  };
  const g = M.phaseGuardrails(summary);
  assert.deepStrictEqual(g.map((x) => x.code), ['phase_design_weak', 'phase_deploy_weak']);
  assert.deepStrictEqual(M.bannerOf(g).map((b) => b.id), ['phase_design_weak', 'phase_deploy_weak']);
  const recs = [{ key: 'ABC-1', type: 'Story', assignee_id: 'u1', status_category: 'done', status: 'Done', created: t0, done_at: t0 + 2 * DAY, eff_done_at: t0 + 2 * DAY, first_active_at: t0, intervals: [{ status: 'In Progress', from: t0, to: t0 + DAY }] }];
  const withPh = M.attachPhases(recs, { prMap: new Map(), tags: [], config: scopeOf([]).config, people: {}, canonical: (x) => x });
  const m = M.computeMetrics(scopeOf(withPh), { since: t0, until: t0 + 14 * DAY }, { tags: [], prs: null });
  assert.ok(m.guardrails.some((x) => x.code === 'phase_dev_weak'));
  for (const v of Object.values(m.phases)) assert.ok(v.coverage >= 0 && v.coverage <= 1);
});

const F = require('../lib/flow.js');
const PR = require('../lib/prs.js');

function richScope(config = {}) {
  const recs = [
    { key: 'ABC-1', type: 'Story', assignee_id: 'u1', status_category: 'done', status: 'Done', created: t0 - DAY, done_at: t0 + 3 * DAY, eff_done_at: t0 + 3 * DAY, first_active_at: t0, first_commit_at: t0 + 3 * DAY, deployed_at: t0 + 6 * DAY, intervals: [{ status: 'In Progress', from: t0, to: t0 + 2 * DAY }], commit_ts: [t0 + DAY] },
    { key: 'ABC-2', type: 'Bug', assignee_id: 'u1', status_category: 'done', status: 'Done', created: t0 + 4 * DAY, done_at: t0 + 6 * DAY, eff_done_at: t0 + 6 * DAY, first_active_at: t0 + 4 * DAY, intervals: [], scope_excluded: true, rework_of: 'ABC-1', rework_confidence: 'high', rework_signals: ['link', 'title_ref'] },
    { key: 'ABC-3', type: 'Task', assignee_id: 'u1', status_category: 'done', status: 'Done', created: t0, done_at: t0 + 5 * DAY, eff_done_at: t0 + 5 * DAY, first_active_at: t0 + DAY, intervals: [], rework_of: 'ABC-1', rework_confidence: 'low', rework_signals: ['title_keyword'] },
    { key: 'ABC-4', type: 'Sub-task', subtask: true, parent_key: 'ABC-1', substantive_subtask: true, credited_to: 'u2', assignee_id: 'u2', commit_ts: [t0 + 2 * DAY] },
  ];
  const withPh = M.attachPhases(recs, { prMap: new Map(), tags: [], config: { ...scopeOf([]).config, ...config }, people: {}, canonical: (x) => x });
  const scope = scopeOf(withPh);
  scope.config = { ...scope.config, ...config };
  return scope;
}
const prs = [{ number: 7, author: 'u1', keys: ['ABC-1'], opened_at: new Date(t0 + DAY).toISOString(), merged_at: new Date(t0 + 2 * DAY).toISOString(), first_review_at: new Date(t0 + DAY + 3600000).toISOString(), first_approval_at: new Date(t0 + DAY + 3600000).toISOString(),
  reviewers: [{ id: 'u2', name: 'u2', approved: true, reviewed_at: new Date(t0 + DAY + 3600000).toISOString() }], comment_authors: { u2: 2 }, additions: 10, deletions: 0, partial: { diff: true } }];

test('contract: every exported flow.js / prs.js metric function appears in computeMetrics output', () => {
  const helpers = new Set(['quantile', 'median', 'bizDaysBetween', 'unplannedReason', 'phaseValues', 'elapsedDays']);
  const flowFns = Object.entries(F).filter(([k, v]) => typeof v === 'function' && !helpers.has(k)).map(([k]) => k);
  const m = M.computeMetrics(richScope(), { since: t0, until: t0 + 14 * DAY }, { tags: [], prs });
  for (const k of flowFns) assert.ok(m.flow[k] && 'value' in m.flow[k], `flow.${k} missing from computeMetrics`);
  const prOut = { prFlowSummary: m.pr_flow, prByPerson: m.pr_people, reviewLoad: m.review_load };
  assert.deepStrictEqual(PR.METRIC_FUNCTIONS.slice().sort(), Object.keys(prOut).sort());
  for (const k of PR.METRIC_FUNCTIONS) {
    assert.strictEqual(typeof PR[k], 'function', `prs.${k} exported`);
    assert.ok(prOut[k], `${k} output present`);
  }
  assert.strictEqual(m.pr_people.u2.reviewed_given, 1);
  assert.strictEqual(m.review_load.top_share, 1);
  assert.strictEqual(m.pr_flow.unreviewed_share, 0);
  assert.ok(m.pr_flow.merges_per_capacity_week > 0);
  assert.ok(m.guardrails.some((g) => g.code === 'pr_partial'));
});

test('rework: rate_all vs rate_strong (0..1), counts by signal, truncation flag', () => {
  const m = M.computeMetrics(richScope(), { since: t0, until: t0 + 14 * DAY }, { tags: [], prs: null });
  assert.strictEqual(m.rework.n, 3);
  assert.strictEqual(m.rework.rate_all, 0.67);
  assert.strictEqual(m.rework.rate_strong, 0.33);
  assert.deepStrictEqual(m.rework.by_signal, { link: 1, title_ref: 1, title_keyword: 1 });
  assert.strictEqual(m.rework.truncated, false);
  for (const v of [m.rework.rate_all, m.rework.rate_strong]) assert.ok(v >= 0 && v <= 1);
});

test('one eligible population: cycleTimeByPhase n equals phaseSummary population', () => {
  const m = M.computeMetrics(richScope(), { since: t0, until: t0 + 14 * DAY }, { tags: [], prs: null });
  assert.strictEqual(m.phase_population, 2, 'excluded rework ticket and sub-task are out');
  assert.strictEqual(m.flow.cycleTimeByPhase.population, 2);
  assert.strictEqual(m.flow.cycleTimeByPhase.value.dev.n, m.phases.dev.n);
});

test('activityOf includes PR review events and substantive sub-task commits', () => {
  const scope = richScope();
  const act = M.activityOf(scope.records, scope, { since: t0, until: t0 + 14 * DAY }, prs);
  assert.ok(act.some((a) => a.person === 'u2' && a.source === 'pr_review' && a.key === 'ABC-1'));
  assert.ok(act.some((a) => a.person === 'u2' && a.key === 'ABC-1' && a.day === t0 + 2 * DAY), 'sub-task commit credited, keyed by parent');
});

test('doraConfig: failure window from side, min_n and hotfix patterns from config, weekend from workweek', () => {
  const c = M.doraConfig({ workweek: [0, 1, 2, 3, 4], failure_window_days: 5, dora_min_n: 4, hotfix_tag_patterns: ['urgent'] }, { bug_window_days: 30 }, { since: 0, until: 1 });
  assert.strictEqual(c.failure_window_days, 30);
  assert.strictEqual(c.min_n, 4);
  assert.deepStrictEqual(c.weekend, [5, 6]);
  assert.deepStrictEqual(c.hotfix_tag_patterns, ['urgent']);
  assert.strictEqual(M.doraConfig({ failure_window_days: 5 }, {}, { since: 0, until: 1 }).failure_window_days, 5);
  assert.strictEqual(M.doraConfig({}, {}, { since: 0, until: 1 }).failure_window_days, 7);
});

test('computeMetrics: a Fri/Sat weekend (Sun–Thu workweek) changes DORA lead time', () => {
  // first commit Thu 2026-01-08, deployed Sun 2026-01-11.
  const thu = Date.UTC(2026, 0, 8, 10);
  const sun = Date.UTC(2026, 0, 11, 10);
  const mk = (workweek) => {
    const recs = [{ key: 'ABC-1', type: 'Story', assignee_id: 'u1', status_category: 'done', status: 'Done', created: thu, done_at: sun, eff_done_at: sun, first_active_at: thu, first_commit_at: thu, deployed_at: sun, deployed_tag: 'v1-deployed', intervals: [] }];
    const scope = scopeOf(recs);
    scope.config = { ...scope.config, workweek };
    return M.computeMetrics(scope, { since: t0, until: t0 + 14 * DAY }, { tags: [{ repo: 'svc', name: 'v1-deployed', sha: 'a', ts: sun }], prs: null }).dora.lead_time.p50;
  };
  assert.notStrictEqual(mk([1, 2, 3, 4, 5]), mk([0, 1, 2, 3, 4]));
});

test('estimate accuracy contract: corpus identity and effective estimate reach the correction action', () => {
  const record = {
    key: 'ABC-1', project: 'ABC', summary: 'Ship migration', type: 'Story',
    assignee_id: 'u1', assignee_name: 'Person One', status_category: 'done',
    done_at: t0 + DAY, eff_done_at: t0 + DAY, manual_days: 8,
  };
  const scope = scopeOf([record]);
  scope.estimates['ABC-1'] = { days: 3, overridden: true };
  const acc = M.computeMetrics(scope, { since: t0, until: t0 + 14 * DAY }).estimate_accuracy;
  assert.deepStrictEqual(acc.worst[0], {
    key: 'ABC-1', project: 'ABC', summary: 'Ship migration', assignee_id: 'u1',
    assignee_name: 'Person One', est_days: 3, actual_days: 8, ratio: 2.667,
  });
  assert.equal(acc.n, 1);
  assert.equal(acc.bins.reduce((sum, b) => sum + b.n, 0), 1);
});
