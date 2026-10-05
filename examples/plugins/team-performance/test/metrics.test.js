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
