// Unit tests for the pure phase model (lib/phases.js). All exact asserts.
'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { phasesFor, phaseSummary, isDeployTag, statusClass, RE_DESIGN } = require('../lib/phases');
const GS = require('../lib/gitscan');

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

test('design: status time; pre-dev activity = distinct workdays outside backlog', () => {
  assert.deepEqual(phasesFor({ intervals: [iv('Design', 0, 1), iv('In Progress', 1, 2)] }).design, { days: 1, evidence: ['status'] });
  const p = phasesFor({
    assignee_id: 'u1',
    intervals: [iv('To Do', 0, 1), iv('In Progress', 2, 3)],
    activity: [
      { at: MON + 0.5 * D, author_id: 'u1' }, // inside To Do → ignored
      { at: MON + 1.2 * D, author_id: 'u1' },
      { at: MON + 1.7 * D, author_id: 'u1' }, // same day → one day
      { at: MON + 1.5 * D, author_id: 'other' },
    ],
  });
  assert.deepEqual(p.design, { days: 1, evidence: ['pre_dev_activity'] });
  // Activity more than 10 workdays before the first dev status does not count.
  const old = phasesFor({ assignee_id: 'u1', intervals: [iv('In Progress', 21, 22)], activity: [{ at: MON + 0.5 * D, author_id: 'u1' }] });
  assert.equal(old.design.days, null);
  assert.equal(old.design.reason, 'not_tracked');
});

test('design: overlapping windows are unioned and clipped before the first dev status', () => {
  const p = phasesFor({
    intervals: [iv('Design', 0, 2), iv('In Progress', 3, 5)],
    subtasks: [{ key: 'ABC-2', type: 'Sub-task', summary: 'Design API', started_at: MON + 1 * D, done_at: MON + 4 * D }],
  });
  assert.deepEqual(p.design, { days: 3, evidence: ['status', 'subtask'] }, 'Mon–Wed once, Thu+ clipped');
});

test('design: linked item timing via lookup; no timing → null + evidence_without_timing', () => {
  const rec = { intervals: [iv('In Progress', 3, 4)], links: [{ key: 'ABC-9', type: 'Spike', summary: 'Queue options' }] };
  const none = phasesFor(rec);
  assert.equal(none.design.days, null);
  assert.equal(none.design.reason, 'evidence_without_timing');
  assert.deepEqual(none.design.evidence, ['linked']);
  const timed = phasesFor(rec, { lookup: (k) => (k === 'ABC-9' ? { started_at: MON, done_at: MON + 2 * D } : null) });
  assert.deepEqual(timed.design, { days: 2, evidence: ['linked'] });
  assert.ok(RE_DESIGN.test('POC for cache') && !RE_DESIGN.test('Write unit tests'));
});

test('review: PR opened Mon, reviewed Tue, merged Wed → pickup 1, in_review 1, total 2', () => {
  const p = phasesFor(
    { intervals: [iv('In Progress', 0, 0.5)], commit_ts: [MON + 0.25 * D] },
    { prs: [{ number: 7, opened_at: MON + 0.5 * D, first_review_at: MON + 1.5 * D, merged_at: MON + 2.5 * D }] },
  );
  assert.deepEqual(p.review, { pickup: 1, in_review: 1, total: 2, source: 'pr', merged: 1, unmerged: 0, prs: [{ number: 7, pickup: 1, in_review: 1, open_to_merge: 2 }] });
  assert.deepEqual(p.dev, { days: 0.5, sources: ['status'] });
  assert.equal(p.coding_days, 0.25);
});

test('review: per-PR — union of windows, median pickup, unmerged counted apart, gap fill per PR window', () => {
  const p = phasesFor(
    // Commit on Thu sits between the two PRs: not inside either PR window → dev gap day.
    { intervals: [], commit_ts: [MON + 3.5 * D] },
    {
      prs: [
        { number: 1, opened_at: MON, first_review_at: MON + 1 * D, merged_at: MON + 2 * D },
        { number: 2, opened_at: MON + 1 * D, first_review_at: MON + 2 * D, merged_at: MON + 3 * D },
        { number: 3, opened_at: MON + 4 * D, state: 'declined' },
      ],
    },
  );
  assert.equal(p.review.total, 3, 'Mon–Wed union, not 2 + 2');
  assert.equal(p.review.pickup, 1);
  assert.equal(p.review.in_review, 2);
  assert.equal(p.review.merged, 2);
  assert.equal(p.review.unmerged, 1);
  assert.deepEqual(p.dev, { days: 1, sources: ['commit_stretch'] });
  const only = phasesFor({}, { prs: [{ opened_at: MON, state: 'open' }] });
  assert.equal(only.review.state, 'unmerged');
  assert.equal(only.review.total, null);
});

test('review: no PR falls back to Code Review status; nothing → nulls', () => {
  assert.deepEqual(phasesFor({ intervals: [iv('In Progress', 0, 1), iv('Code Review', 1, 3)] }).review,
    { pickup: null, in_review: 2, total: 2, source: 'status', merged: 0, unmerged: 0, prs: [] });
  assert.deepEqual(phasesFor({ intervals: [] }).review, { pickup: null, in_review: null, total: null, source: null, merged: 0, unmerged: 0, prs: [] });
});

test('deploy: reachability (deployed_at) is primary; done_git_at without a PR', () => {
  const prs = [{ opened_at: MON, merged_at: MON + D, repo: 'svc' }];
  const p = phasesFor({ deployed_at: MON + 3 * D, deployed_tag: 'v1-deployed' }, { prs, tags: [{ name: 'hf-1', at: MON + 2 * D, repo: 'svc' }] });
  assert.deepEqual(p.deploy, { days: 2, tag: 'v1-deployed', source: 'reachability' });
  const noPr = phasesFor({ done_git_at: MON, deployed_at: new Date(MON + D).toISOString(), deployed_tag: 'rel-HF2' });
  assert.deepEqual(noPr.deploy, { days: 1, tag: 'rel-HF2', source: 'reachability' });
});

test('deploy: fallback only with a tag of the SAME repo; otherwise not_deployed', () => {
  const prs = [{ opened_at: MON, merged_at: MON + D, repo: 'svc' }];
  const tags = [{ name: 'v1.2', at: MON + 2 * D, repo: 'svc' }, { name: 'other-deployed', at: MON + 2 * D, repo: 'web' }, { name: 'release-HF-3', at: MON + 4 * D, repo: 'svc' }];
  assert.deepEqual(phasesFor({}, { prs, tags }).deploy, { days: 3, tag: 'release-HF-3', source: 'tag_same_repo' });
  assert.deepEqual(phasesFor({}, { prs, tags: tags.slice(0, 2) }).deploy, { days: null, reason: 'not_deployed' });
  assert.deepEqual(phasesFor({}, { prs }).deploy, { days: null, reason: 'not_deployed' });
  // Commit repos count too (no-PR ticket).
  assert.equal(phasesFor({ done_git_at: MON, repos: ['web'] }, { tags }).deploy.tag, 'other-deployed');
  // A "Ready for Deploy" status window feeds the phase when no tag says more.
  assert.deepEqual(phasesFor({ intervals: [iv('In Progress', 0, 1), iv('Ready for Deploy', 1, 3)] }).deploy, { days: 2, tag: null, source: 'status' });
});

test('deploy tag rule: phases and gitscan give identical results', () => {
  const table = { 'hotfixes-2026': true, prodHotfix1: true, v2hf: true, 'rel-HF2': true, 'v1-DEPLOYED': true, 'release-1.2': false };
  for (const [n, want] of Object.entries(table)) {
    assert.equal(isDeployTag(n), want, n);
    assert.equal(GS.isDeployTag(n), want, n);
  }
  assert.equal(isDeployTag, GS.isDeployTag, 'one rule, re-exported');
});

test('status classes: anchored backlog, reopen = dev, deploy_wait', () => {
  const c = (status) => statusClass({ status });
  assert.equal(c('To Do'), 'backlog');
  assert.equal(c('To-Do'), 'backlog');
  assert.equal(c('Selected for Development'), 'backlog');
  assert.equal(c('Reopened'), 'dev');
  assert.equal(c('Re-open'), 'dev');
  assert.equal(c('Ready for Deploy'), 'deploy_wait');
  assert.equal(c('Ready for Release'), 'deploy_wait');
  assert.equal(c('Ready for QA'), 'qa');
  assert.equal(c('Open PR'), 'review');
  assert.equal(c('In Progress'), 'dev');
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

test('qa: threshold met → commit days are rework, remainder wait; never the whole interval', () => {
  const base = [iv('In Progress', 0, 1), iv('QA', 1, 4)];
  const run = (commit_ts) => phasesFor({ intervals: base, commit_ts }, { cfg: { qa_work_min_commit_days: 2 } });
  const one = run([MON + 1.5 * D]);
  assert.deepEqual(one.qa, { wait: 3, rework: 0, counted: false });
  assert.equal(one.dev.days, 1);
  const sameDay = run([MON + 1.2 * D, MON + 1.8 * D]);
  assert.deepEqual(sameDay.qa, { wait: 3, rework: 0, counted: false });
  const twoDays = run([MON + 1.5 * D, MON + 2.5 * D]);
  assert.deepEqual(twoDays.qa, { wait: 1, rework: 2, counted: true });
  assert.deepEqual(twoDays.dev, { days: 3, sources: ['status', 'qa_rework'] });
  const outside = run([MON + 1.5 * D, MON + 7.5 * D]);
  assert.deepEqual(outside.qa, { wait: 3, rework: 0, counted: false });
});

test('qa / rework: no QA stage or no blame → null with a reason (never a fake 0)', () => {
  const p = phasesFor({ intervals: [iv('In Progress', 0, 1)] });
  assert.deepEqual(p.qa, { wait: null, rework: null, counted: false, reason: 'no_qa_stage' });
  assert.deepEqual(p.rework, { in_days: null, out_days: null, reason: 'no_git_blame' });
});

test('tz: 23:30 local in UTC+3 lands on the local day (QA distinct days)', () => {
  const from = U('2026-01-04T21:00:00Z');
  const to = U('2026-01-06T21:00:00Z');
  const rec = { intervals: [{ status: 'QA', from, to }], commit_ts: [U('2026-01-05T20:30:00Z'), U('2026-01-05T21:30:00Z')] };
  const local = phasesFor(rec, { tz: 'Europe/Moscow', cfg: { qa_work_min_commit_days: 2 } });
  assert.deepEqual(local.qa, { wait: 0, rework: 2, counted: true });
  const utc = phasesFor(rec, { tz: 'UTC', cfg: { qa_work_min_commit_days: 2 } });
  assert.equal(utc.qa.counted, false);
});

test('rework passthrough and phaseSummary: n/coverage over non-null, coverage is a 0..1 fraction', () => {
  const a = phasesFor({ intervals: [iv('Design', 0, 1), iv('In Progress', 1, 3)], rework: { in_days: 1.5, out_days: 0 } });
  const b = phasesFor({ intervals: [iv('In Progress', 0, 4)] });
  const c = phasesFor({ intervals: [iv('In Progress', 0, 1)] });
  assert.deepEqual(a.rework, { in_days: 1.5, out_days: 0 });
  const s = phaseSummary([{ phases: a }, { phases: b }, c]);
  assert.deepEqual(s.design, { n: 1, p50: 1, p75: 1, coverage: 0.333 });
  assert.deepEqual(s.dev, { n: 3, p50: 2, p75: 3, coverage: 1 });
  assert.deepEqual(s.deploy, { n: 0, p50: null, p75: null, coverage: 0 });
  assert.deepEqual(s.qa_wait, { n: 0, p50: null, p75: null, coverage: 0 });
  assert.equal(s.rework_in.n, 1);
  for (const [name, v] of Object.entries(s)) assert.ok(v.coverage >= 0 && v.coverage <= 1, `${name} coverage ${v.coverage}`);
});
