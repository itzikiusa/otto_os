// Unit tests for lib/flow.js — exact values on a small fixture.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const F = require('../lib/flow.js');

const T = (s) => Date.parse(s);
const W = { since: T('2026-06-01T00:00:00Z'), until: T('2026-06-15T00:00:00Z') }; // 10 business days = 2 weeks
const NOW = T('2026-06-15T00:00:00Z');

const R = [
  { key: 'ABC-1', type: 'Story', epic: 'ABC-100', labels: ['api'], assignee_id: 'a', priority: 'Medium', created_at: T('2026-05-01T00:00:00Z'), started_at: T('2026-06-01T09:00:00Z'), done_at: T('2026-06-03T09:00:00Z'), dev_days: 2, estimate_days: 2, actual_days: 2, phases: { design: 1, dev: 2, review: 0.5, deployment: 1, rework: 0 } },
  { key: 'ABC-2', type: 'Story', epic: 'ABC-100', labels: ['api', 'ui'], assignee_id: 'a', priority: 'Medium', created_at: T('2026-05-01T00:00:00Z'), started_at: T('2026-06-02T09:00:00Z'), done_at: T('2026-06-08T09:00:00Z'), dev_days: 4, estimate_days: 2, actual_days: 4, phases: { design: null, dev: 4, review: 1, deployment: 2, rework: 1 } },
  { key: 'ABC-3', type: 'Bug', assignee_id: 'b', priority: 'High', created_at: T('2026-06-02T00:00:00Z'), started_at: T('2026-06-02T10:00:00Z'), done_at: T('2026-06-04T10:00:00Z'), dev_days: 1, estimate_days: 4, actual_days: 1, phases: { dev: 1, review: 0.5 } },
  { key: 'ABC-4', type: 'Story', assignee_id: 'b', priority: 'Highest', created_at: T('2026-05-01T00:00:00Z'), started_at: T('2026-06-03T09:00:00Z'), done_at: T('2026-06-05T09:00:00Z'), dev_days: 3, estimate_days: 3, actual_days: 3.3 },
  { key: 'ABC-5', type: 'Task', assignee_id: 'b', priority: 'Medium', created_at: T('2026-06-03T00:00:00Z'), started_at: T('2026-06-04T09:00:00Z'), done_at: T('2026-06-10T09:00:00Z'), dev_days: 2, estimate_days: 1, actual_days: 0.4 },
  { key: 'ABC-6', type: 'Story', assignee_id: 'a', scope_excluded: true, started_at: T('2026-06-01T09:00:00Z'), done_at: T('2026-06-02T09:00:00Z'), dev_days: 9, estimate_days: 9, actual_days: 9 },
  { key: 'ABC-7', type: 'Story', assignee_id: 'a', created_at: T('2026-05-01T00:00:00Z'), started_at: T('2026-06-01T09:00:00Z') }, // open, 10 bdays old
  { key: 'ABC-8', type: 'Story', assignee_id: 'b', created_at: T('2026-05-01T00:00:00Z'), started_at: T('2026-06-12T09:00:00Z') }, // open, 1 bday old
];

test('throughputPerWeek: counts and weights, excluding scope_excluded', () => {
  const m = F.throughputPerWeek(R, W);
  assert.deepEqual(m.value, { count: 5, weighted: 12, weeks: 2, count_per_week: 2.5, weighted_per_week: 6 });
  assert.equal(m.n, 5);
  assert.equal(m.coverage, 0.833);
  assert.deepEqual(m.weak_reasons, []);
});

test('wip and agingWip', () => {
  const w = F.wip(R, { now: NOW });
  assert.deepEqual(w.value, { count: 2, by_person: { a: 1, b: 1 }, per_person_mean: 1 });
  const a = F.agingWip(R, { now: NOW });
  // Story cycles (bdays): ABC-1 2, ABC-2 4, ABC-4 2 → p75 = 3
  assert.equal(a.value.p75_by_bucket.Story, 3);
  assert.equal(a.value.count, 1);
  assert.deepEqual(a.value.items[0], { key: 'ABC-7', bucket: 'Story', assignee_id: 'a', age_days: 10, p75_days: 3 });
});

test('contextSwitching: mean distinct keys per person-day', () => {
  const act = [
    { person: 'a', day: '2026-06-01', key: 'ABC-1' }, { person: 'a', day: '2026-06-01', key: 'ABC-2' }, { person: 'a', day: '2026-06-01', key: 'ABC-1' },
    { person: 'a', day: '2026-06-02', key: 'ABC-2' },
    { person: 'b', day: '2026-06-01', key: 'ABC-3' }, { person: 'b', day: '2026-06-01', key: 'ABC-4' }, { person: 'b', day: '2026-06-01', key: 'ABC-5' },
  ];
  const m = F.contextSwitching(act);
  assert.equal(m.value, 2); // (2 + 1 + 3) / 3
  assert.equal(m.n, 3);
  assert.deepEqual(m.by_person, { a: { active_days: 2, mean_keys_per_day: 1.5 }, b: { active_days: 1, mean_keys_per_day: 3 } });
  assert.ok(m.weak_reasons.includes('low_n'));
});

test('investmentMix by type / epic / label', () => {
  const m = F.investmentMix(R, W);
  assert.equal(m.value.total_dev_days, 12);
  assert.deepEqual(m.value.by_type.Story, { days: 9, share: 0.75 });
  assert.deepEqual(m.value.by_type.Bug, { days: 1, share: 0.083 });
  assert.deepEqual(m.value.by_epic['ABC-100'], { days: 6, share: 0.5 });
  assert.deepEqual(m.value.by_label.api, { days: 6, share: 0.5 });
  assert.deepEqual(m.value.by_label['no label'], { days: 6, share: 0.5 });
});

test('unplannedShare: bug, highest priority, interrupt', () => {
  const m = F.unplannedShare(R, W);
  // pool: ABC-1..5, 7, 8 = 7; unplanned: ABC-3 bug, ABC-4 highest, ABC-5 interrupt
  assert.equal(m.n, 7);
  assert.equal(m.value, 0.429);
  assert.deepEqual(m.by_reason, { bug: 1, highest_priority: 1, interrupt: 1 });
  assert.deepEqual(m.keys, ['ABC-3', 'ABC-4', 'ABC-5']);
});

test('estimateAccuracy histogram and ±25%', () => {
  const m = F.estimateAccuracy(R, W);
  // ratios: 1, 2, 0.25, 1.1, 0.4
  assert.deepEqual(m.value.histogram, { '<0.5': 2, '0.5-0.8': 0, '0.8-1.25': 2, '1.25-2': 0, '>2': 1 });
  assert.equal(m.value.pct_within_25, 0.4);
  assert.equal(m.value.median_ratio, 1);
});

test('cycleTimeByPhase: medians, design not tracked is null not 0', () => {
  const m = F.cycleTimeByPhase(R, W);
  assert.equal(m.n, 3);
  assert.deepEqual(m.value.dev, { median: 2, p75: 3, n: 3, coverage: 1 });
  assert.deepEqual(m.value.design, { median: 1, p75: 1, n: 1, coverage: 0.333 });
  assert.deepEqual(m.value.review, { median: 0.5, p75: 0.75, n: 3, coverage: 1 });
  // total = median per-ticket elapsed (start → done) over ALL 5 done items,
  // not a sum of phases: 2, 4, 2, 2, 4 business days
  assert.deepEqual(m.value.total, { median: 2, p75: 4, n: 5, coverage: 1 });
  assert.equal(m.value.qa_wait.status, 'not_tracked');
  assert.equal(m.value.qa_wait.median, null);
  for (const ph of ['coding', 'review_pickup', 'qa_wait']) assert.ok(F.PHASES.includes(ph));
  assert.ok(m.weak_reasons.includes('design_not_tracked_share'));
  const empty = F.cycleTimeByPhase([{ key: 'ABC-9', done_at: T('2026-06-02T00:00:00Z'), phases: { dev: 1 } }], W);
  assert.equal(empty.value.design.median, null);
  assert.equal(empty.value.design.status, 'not_tracked');
});

test('zero denominators give null', () => {
  const none = { since: T('2026-06-06T00:00:00Z'), until: T('2026-06-08T00:00:00Z') }; // weekend only
  const m = F.throughputPerWeek(R, none, { people: { a: {} } });
  assert.equal(m.value.count_per_week, null);
  assert.equal(m.per_person.a.count_per_capacity_day, null);
  assert.equal(m.coverage, null);
  assert.equal(F.unplannedShare([], W).value, null);
  assert.equal(F.estimateAccuracy([], W).value.pct_within_25, null);
  assert.equal(F.contextSwitching([]).value, null);
});

// ---- P5 expansion ---------------------------------------------------------

// phasesFor()-shaped record: dev 4 includes 1 day of QA rework.
const PF = (o) => ({
  key: 'ABC-20', type: 'Story', assignee_id: 'a', started_at: T('2026-06-01T09:00:00Z'), done_at: T('2026-06-11T09:00:00Z'), dev_days: 4,
  phases: {
    design: { days: null, evidence: [], reason: 'not_tracked' }, dev: { days: 4, sources: ['status'] },
    review: { pickup: 1, in_review: 1, total: 2, source: 'pr' }, qa: { wait: 2, rework: 1, counted: true },
    deploy: { days: 1, tag: 'deployed-1' }, rework: { in_days: 0.5, out_days: 0 },
  },
  ...o,
});

test('phaseValues reads phasesFor() output; null stays null', () => {
  const v = F.phaseValues(PF());
  assert.deepEqual(v, { design: null, coding: 3, dev: 4, review_pickup: 1, in_review: 1, review: 2, qa_wait: 2, qa_rework: 1, deployment: 1, rework: 0.5 });
  assert.equal(F.phaseValues({}), null);
});

test('cycleTimeByPhase: deployed_at ends elapsed; unknown start is uncovered, never 0', () => {
  const recs = [
    PF({ deployed_at: T('2026-06-12T09:00:00Z') }), // 6/1→6/12 = 9 bdays
    PF({ key: 'ABC-21', started_at: null }),
  ];
  const m = F.cycleTimeByPhase(recs, W);
  assert.deepEqual(m.value.total, { median: 9, p75: 9, n: 1, coverage: 0.5 });
  assert.equal(m.value.coding.median, 3);
  assert.equal(m.value.review_pickup.median, 1);
  assert.equal(m.value.design.status, 'not_tracked');
  assert.ok(m.weak_reasons.includes('no_start_time'));
  assert.deepEqual(m.guardrail.level, 'weak');
});

test('flowEfficiency = (coding + in_review + qa_rework) / elapsed', () => {
  const m = F.flowEfficiency([PF()], W); // elapsed 6/1→6/11 = 8 bdays; active 3+1+1 = 5
  assert.equal(m.value, 0.625);
  assert.equal(m.active_days, 5);
  assert.equal(m.elapsed_days, 8);
  assert.equal(m.n, 1);
  assert.ok(m.guardrail.reasons.includes('low_n'));
  assert.equal(F.flowEfficiency([], W).value, null);
});

test('focusShare: days with <= 1 key over active days', () => {
  const act = [
    { person: 'a', day: '2026-06-01', key: 'ABC-1' }, { person: 'a', day: '2026-06-01', key: 'ABC-2' },
    { person: 'a', day: '2026-06-02', key: 'ABC-2' }, { person: 'a', day: '2026-06-02', key: 'ABC-2' },
    { person: 'b', day: '2026-06-01', key: 'ABC-3' },
  ];
  const m = F.focusShare(act);
  assert.equal(m.value, 0.667);
  assert.deepEqual(m.by_person.a, { active_days: 2, focused_days: 1, focus_share: 0.5 });
  assert.equal(F.focusShare([]).value, null);
});

test('escapeRate: bugs pointing back at items delivered in the window', () => {
  const recs = [
    { key: 'ABC-30', type: 'Story', done_at: T('2026-06-02T00:00:00Z'), deployed_at: T('2026-06-03T00:00:00Z') },
    { key: 'ABC-31', type: 'Story', done_at: T('2026-06-04T00:00:00Z') },
    { key: 'ABC-32', type: 'Bug', rework_of: 'ABC-30', done_at: T('2026-06-20T00:00:00Z') },
    { key: 'ABC-33', type: 'Task', rework_of: 'ABC-31' }, // not a bug → not an escape
    { key: 'ABC-34', type: 'Bug', rework_of: 'ABC-99' }, // origin outside window
  ];
  const m = F.escapeRate(recs, W);
  assert.equal(m.n, 2); // ABC-30, 31 (ABC-32 delivered outside W)
  assert.equal(m.value, 0.5);
  assert.deepEqual(m.items, [{ key: 'ABC-30', bugs: ['ABC-32'] }]);
  assert.ok(F.escapeRate([{ key: 'ABC-1', done_at: T('2026-06-02T00:00:00Z') }], W).weak_reasons.includes('no_rework_links'));
});

test('sprintPlanning: planned at start vs done at end, mid-sprint adds = creep', () => {
  const S1 = { name: 'Sprint 1', start: T('2026-06-01T08:00:00Z'), end: T('2026-06-12T18:00:00Z') };
  const recs = [
    { key: 'ABC-40', created_at: T('2026-05-20T00:00:00Z'), sprint_changes: [{ at: T('2026-05-28T00:00:00Z'), from: '', to: 'Sprint 1' }], done_at: T('2026-06-05T00:00:00Z') },
    { key: 'ABC-41', created_at: T('2026-05-20T00:00:00Z'), sprint_changes: [{ at: T('2026-05-28T00:00:00Z'), from: '', to: 'Sprint 1' }, { at: T('2026-06-13T00:00:00Z'), from: 'Sprint 1', to: 'Sprint 1, Sprint 2' }] },
    { key: 'ABC-42', created_at: T('2026-06-03T00:00:00Z'), sprint_changes: [{ at: T('2026-06-03T00:00:00Z'), from: '', to: 'Sprint 1' }], done_at: T('2026-06-10T00:00:00Z') },
    { key: 'ABC-43', created_at: T('2026-05-20T00:00:00Z'), sprint_changes: [{ at: T('2026-06-08T00:00:00Z'), from: '', to: ['Sprint 1'] }] },
    { key: 'ABC-44', created_at: T('2026-05-20T00:00:00Z'), sprints: ['Sprint 2'] },
  ];
  const m = F.sprintPlanning(recs, [S1]);
  const row = m.value.sprints[0];
  assert.equal(row.planned, 2);
  assert.equal(row.planned_done, 1);
  assert.equal(row.added, 2);
  assert.equal(row.added_done, 1);
  assert.deepEqual(row.added_keys, ['ABC-42', 'ABC-43']);
  assert.deepEqual(row.carried_over, ['ABC-41']);
  assert.equal(m.value.accuracy, 0.5);
  assert.equal(m.value.scope_creep, 1);
  assert.ok(F.sprintPlanning([], [S1]).weak_reasons.includes('no_sprint_history'));
  assert.equal(F.sprintPlanning([], [S1]).value.accuracy, null);
});

test('reworkRateByPerson: share reworked + rework days per dev day', () => {
  const recs = [
    PF({ key: 'ABC-50' }), PF({ key: 'ABC-51' }),
    { key: 'ABC-52', type: 'Bug', rework_of: 'ABC-50' },
  ];
  const m = F.reworkRateByPerson(recs, W);
  assert.deepEqual(m.by_person.a, {
    delivered: 2, reworked: 1, keys: ['ABC-50'], reworked_share: 0.5, rework_days: 1, dev_days: 8, rework_days_ratio: 0.125,
    guardrail: { level: 'weak', reasons: ['low_n'] },
  });
  assert.equal(m.value, 0.5);
});

test('investmentByEpic rows with summaries, no_epic + top unlinked tickets', () => {
  const m = F.investmentByEpic(R, W, { 'ABC-100': 'Payments revamp' });
  // equal days → ordered by key
  assert.deepEqual(m.value.rows[0], { epic_key: 'ABC-100', epic_summary: 'Payments revamp', dev_days: 6, share: 0.5 });
  assert.deepEqual(m.value.rows[1], { epic_key: 'no_epic', epic_summary: null, dev_days: 6, share: 0.5 });
  assert.deepEqual(m.value.no_epic.top_tickets.map((t) => t.key), ['ABC-4', 'ABC-5', 'ABC-3']);
  assert.ok(m.weak_reasons.includes('high_unlinked_share'));
});

test('unplannedShare lists tickets; estimateAccuracy guardrail under 10 samples', () => {
  const u = F.unplannedShare(R, W);
  assert.deepEqual(u.tickets.map((t) => [t.key, t.reason]), [['ABC-4', 'highest_priority'], ['ABC-5', 'interrupt'], ['ABC-3', 'bug']]);
  const e = F.estimateAccuracy(R, W);
  assert.ok(e.weak_reasons.includes('low_sample'));
  assert.deepEqual(e.guardrail.level, 'weak');
  const many = Array.from({ length: 10 }, (_, i) => ({ key: `ABC-${60 + i}`, done_at: T('2026-06-02T00:00:00Z'), estimate_days: 1, actual_days: 1 }));
  assert.ok(!F.estimateAccuracy(many, W).weak_reasons.includes('low_sample'));
  assert.equal(F.estimateAccuracy(many, W).guardrail, null);
});

test('estimate accuracy contract: correction candidates and chart bins share the eligible population', () => {
  const extra = [
    { ...R[0], key: 'ZERO-1', estimate_days: 2, actual_days: 0 },
    { ...R[0], key: 'BAD-1', estimate_days: Infinity },
    { ...R[0], key: 'BAD-2', actual_days: null },
    { ...R[0], key: 'BAD-3', estimate_days: 0 },
    { ...R[0], key: 'CHILD-1', subtask: true, actual_days: 100 },
    { ...R[0], key: 'OLD-1', done_at: W.since - 1, actual_days: 100 },
  ];
  const m = F.estimateAccuracy([...R, ...extra], W);
  assert.deepEqual(m.value.worst.map((r) => r.key), ['ZERO-1', 'ABC-3', 'ABC-5', 'ABC-2']);
  assert.equal(m.value.worst[0].ratio, 0, 'zero actual remains observed data');
  assert.equal(m.value.worst[1].est_days, 4);
  assert.equal(m.value.worst[1].actual_days, 1);
  assert.equal(m.value.n, 6);
  assert.equal(m.value.bins.reduce((sum, b) => sum + b.n, 0), m.n);
  assert.deepEqual(m.value.bins.map((b) => b.n), Object.values(m.value.histogram));
  assert.equal(m.value.within_25, m.value.pct_within_25);
  assert.equal(m.value.pct_within_25, 0.333);
});

test('estimate accuracy contract: candidates are bounded and ties deterministic; empty data is explicit', () => {
  const records = Array.from({ length: 45 }, (_, i) => ({ ...R[0], key: `ABC-${String(i).padStart(3, '0')}`, actual_days: 8 }));
  const worst = F.estimateAccuracy(records.reverse(), W).value.worst;
  assert.equal(worst.length, 30);
  assert.equal(worst[0].key, 'ABC-000');
  assert.equal(worst[29].key, 'ABC-029');
  const empty = F.estimateAccuracy([], W).value;
  assert.deepEqual(empty.worst, []);
  assert.equal(empty.n, 0);
  assert.equal(empty.within_25, null);
  assert.ok(empty.bins.every((b) => b.n === 0));
});
