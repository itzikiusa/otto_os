// Unit tests for the pure phase model (lib/phases.js). All exact asserts.
'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { phasesFor, phaseSummary, isDeployTag } = require('../lib/phases');

const U = (s) => Date.parse(s);
// 2026-01-05 is a Monday.
const MON = U('2026-01-05T00:00:00Z');
const D = 86400000;
const iv = (status, fromDay, toDay) => ({ status, from: MON + fromDay * D, to: MON + toDay * D });

test('design: no evidence → null + not_tracked (never a fake 0)', () => {
  const p = phasesFor({ intervals: [iv('In Progress', 0, 2)] });
  assert.equal(p.design.days, null);
  assert.equal(p.design.reason, 'not_tracked');
  assert.deepEqual(p.design.evidence, []);
});

test('design: a spike sub-task gives its exact working days', () => {
  const p = phasesFor({
    intervals: [iv('In Progress', 3, 4)],
    subtasks: [
      { key: 'ABC-2', type: 'Sub-task', summary: 'Spike: evaluate queue options', started_at: MON, done_at: MON + 2 * D },
      { key: 'ABC-3', type: 'Sub-task', summary: 'Write unit tests', started_at: MON, done_at: MON + 3 * D },
    ],
  });
  assert.equal(p.design.days, 2);
  assert.deepEqual(p.design.evidence, ['subtask']);
  assert.equal(p.design.reason, undefined);
});

test('design: status time and pre-dev assignee activity', () => {
  assert.deepEqual(phasesFor({ intervals: [iv('Design', 0, 1), iv('In Progress', 1, 2)] }).design, { days: 1, evidence: ['status'] });
  const p = phasesFor({
    assignee_id: 'u1',
    intervals: [iv('To Do', 0, 2), iv('In Progress', 2, 3)],
    activity: [{ at: MON + 1 * D, author_id: 'u1' }, { at: MON, author_id: 'other' }],
  });
  assert.deepEqual(p.design, { days: 1, evidence: ['pre_dev_activity'] });
});

test('review: PR opened Mon, reviewed Tue, merged Wed → pickup 1, in_review 1, total 2', () => {
  const p = phasesFor(
    { intervals: [iv('In Progress', 0, 0.5)] },
    { prs: [{ opened_at: MON + 0.5 * D, first_review_at: MON + 1.5 * D, merged_at: MON + 2.5 * D }] },
  );
  assert.deepEqual(p.review, { pickup: 1, in_review: 1, total: 2, source: 'pr' });
  assert.deepEqual(p.dev, { days: 0.5, sources: ['status'] });
});

test('review: no PR falls back to Code Review status; nothing → nulls', () => {
  assert.deepEqual(phasesFor({ intervals: [iv('In Progress', 0, 1), iv('Code Review', 1, 3)] }).review,
    { pickup: null, in_review: 2, total: 2, source: 'status' });
  assert.deepEqual(phasesFor({ intervals: [] }).review, { pickup: null, in_review: null, total: null, source: null });
});

test('deploy: hf tag gives exact days; non-deploy tag or no tag → null', () => {
  const prs = [{ opened_at: MON, merged_at: MON + D }];
  const hf = phasesFor({}, { prs, tags: [{ name: 'v1.2', at: MON + 2 * D }, { name: 'release-HF-3', at: MON + 4 * D }] });
  assert.deepEqual(hf.deploy, { days: 3, tag: 'release-HF-3' });
  assert.equal(phasesFor({}, { prs, tags: [{ name: 'v1.2', at: MON + 2 * D }] }).deploy.days, null);
  assert.equal(phasesFor({}, { prs }).deploy.days, null);
  assert.equal(isDeployTag('prod_Deployed_2026'), true);
  assert.equal(isDeployTag('hotfix-1'), true);
  assert.equal(isDeployTag('v2.0.0'), false);
});

test('dev: To-Do / backlog time is excluded', () => {
  const p = phasesFor({ intervals: [iv('To Do', 0, 3), iv('Backlog', 3, 4), iv('In Progress', 7, 9)] });
  assert.deepEqual(p.dev, { days: 2, sources: ['status'] });
});

test('dev: commit days outside status windows fill gaps', () => {
  const p = phasesFor({
    intervals: [iv('To Do', 0, 2), iv('In Progress', 2, 3)],
    commit_ts: [MON + 0.5 * D, MON + 0.6 * D, MON + 1.5 * D, MON + 2.5 * D, MON + 5.5 * D /* Sat */],
  });
  assert.deepEqual(p.dev, { days: 3, sources: ['status', 'commit_stretch'] });
});

test('qa: threshold 2 distinct commit days', () => {
  const base = [iv('In Progress', 0, 1), iv('QA', 1, 4)];
  const run = (commit_ts) => phasesFor({ intervals: base, commit_ts }, { cfg: { qa_work_min_commit_days: 2 } });
  const one = run([MON + 1.5 * D]);
  assert.deepEqual(one.qa, { wait: 3, rework: 0, counted: false });
  assert.equal(one.dev.days, 1);
  const sameDay = run([MON + 1.2 * D, MON + 1.8 * D]);
  assert.deepEqual(sameDay.qa, { wait: 3, rework: 0, counted: false });
  const twoDays = run([MON + 1.5 * D, MON + 2.5 * D]);
  assert.deepEqual(twoDays.qa, { wait: 0, rework: 3, counted: true });
  assert.equal(twoDays.dev.days, 4);
  // One commit in QA + one after QA: the out-of-window one doesn't count toward QA.
  const outside = run([MON + 1.5 * D, MON + 7.5 * D]);
  assert.deepEqual(outside.qa, { wait: 3, rework: 0, counted: false });
});

test('tz: 23:30 local in UTC+3 lands on the local day (QA distinct days)', () => {
  // QA Mon 00:00 → Wed 00:00 local Moscow. Commits: Mon 23:30 local (=20:30Z) and Tue 00:30 local (=21:30Z Mon).
  const from = U('2026-01-04T21:00:00Z');
  const to = U('2026-01-06T21:00:00Z');
  const rec = { intervals: [{ status: 'QA', from, to }], commit_ts: [U('2026-01-05T20:30:00Z'), U('2026-01-05T21:30:00Z')] };
  const local = phasesFor(rec, { tz: 'Europe/Moscow', cfg: { qa_work_min_commit_days: 2 } });
  assert.deepEqual(local.qa, { wait: 0, rework: 2, counted: true });
  const utc = phasesFor(rec, { tz: 'UTC', cfg: { qa_work_min_commit_days: 2 } });
  assert.equal(utc.qa.counted, false);
});

test('rework passthrough and phaseSummary over non-null values', () => {
  const a = phasesFor({ intervals: [iv('Design', 0, 1), iv('In Progress', 1, 3)], rework: { in_days: 1.5, out_days: 0 } });
  const b = phasesFor({ intervals: [iv('In Progress', 0, 4)] });
  const c = phasesFor({ intervals: [iv('In Progress', 0, 1)] });
  assert.deepEqual(a.rework, { in_days: 1.5, out_days: 0 });
  const s = phaseSummary([{ phases: a }, { phases: b }, c]);
  assert.deepEqual(s.design, { n: 1, p50: 1, p75: 1, coverage: 33 });
  assert.deepEqual(s.dev, { n: 3, p50: 2, p75: 3, coverage: 100 });
  assert.deepEqual(s.deploy, { n: 0, p50: null, p75: null, coverage: 0 });
});
