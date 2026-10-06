// Sub-task attribution (lib/subtasks.js). Pure — no I/O.
// Run (from the plugin dir): node --test
const { test } = require('node:test');
const assert = require('node:assert/strict');

const S = require('../lib/subtasks.js');

// 2026-06-01 is a Monday (UTC).
const T = (s) => Date.parse(s);
const iv = (from, to) => ({ status: 'In Progress', phase: 'implementation', from: T(from), to: T(to) });
const story = (o) => ({ key: 'ABC-1', subtask: false, type: 'Story', summary: 'Story', assignee_id: 'u-a', dev_days: 3, intervals: [iv('2026-06-01T00:00:00Z', '2026-06-04T00:00:00Z')], git_authors: [{ name: 'A', email: 'a@x', commits: 2 }], ...o });
const sub = (o) => ({ subtask: true, parent_key: 'ABC-1', type: 'Sub-task', summary: 'Step', assignee_id: 'u-a', dev_days: 0, intervals: [], git_authors: [], ...o });
const by = (rs) => Object.fromEntries(rs.map((r) => [r.key, r]));

test('checklist sub-task (same person, no substance) rolls up, is not credited', () => {
  const out = by(S.classifySubtasks([story(), sub({ key: 'ABC-2', summary: 'Write unit tests' })]));
  assert.equal(out['ABC-2'].rollup, true);
  assert.equal(out['ABC-2'].substantive_subtask, undefined);
  assert.equal(out['ABC-2'].credited_to, undefined);
  assert.equal(out['ABC-1'].substantive_subtasks, undefined);
});

test('same person WITH substance under a timed story is still a checklist', () => {
  const out = by(S.classifySubtasks([story(), sub({ key: 'ABC-2', dev_days: 2, commit_ts: [T('2026-06-02T10:00:00Z')] })]));
  assert.equal(out['ABC-2'].rollup, true);
});

test("B's real sub-task under A's story is credited to B and kept out of the rollup", () => {
  const b = sub({
    key: 'ABC-3', assignee_id: 'u-b', assignee_name: 'Dev B', dev_days: 2,
    commit_ts: [T('2026-06-08T10:00:00Z'), T('2026-06-09T10:00:00Z')],
    intervals: [iv('2026-06-08T00:00:00Z', '2026-06-10T00:00:00Z')],
    git_authors: [{ name: 'B', email: 'b@x', commits: 2 }],
  });
  const out = by(S.classifySubtasks([story(), b]));
  assert.equal(out['ABC-3'].rollup, false);
  assert.equal(out['ABC-3'].substantive_subtask, true);
  assert.equal(out['ABC-3'].credited_to, 'u-b');
  assert.equal(out['ABC-3'].assignee_id, 'u-b');
  assert.deepEqual(out['ABC-1'].substantive_subtasks, ['ABC-3']);
  assert.equal(out['ABC-1'].child_dev_days, undefined, "B's days never land on A's story");
  assert.deepEqual(out['ABC-1'].git_authors.map((a) => a.name), ['A']);
});

test('other person but no substance (tiny checklist) rolls up', () => {
  const out = by(S.classifySubtasks([story(), sub({ key: 'ABC-3', assignee_id: 'u-b', dev_days: 0.2 })]));
  assert.equal(out['ABC-3'].rollup, true);
});

test('story with no own timing adopts its sub-task\'s timing', () => {
  const p = story({ dev_days: 0, intervals: [], git_authors: [] });
  const s = sub({
    key: 'ABC-2', dev_days: 2, commit_ts: [T('2026-06-02T10:00:00Z')], first_commit_at: T('2026-06-02T10:00:00Z'),
    intervals: [iv('2026-06-01T00:00:00Z', '2026-06-03T00:00:00Z')],
    git_authors: [{ name: 'A', email: 'a@x', commits: 1 }],
  });
  const out = by(S.classifySubtasks([p, s]));
  assert.equal(out['ABC-2'].substantive_subtask, true);
  assert.equal(out['ABC-2'].rollup, true, 'same person: one unit, credited once via the story');
  assert.equal(out['ABC-1'].dev_days, 2);
  assert.equal(out['ABC-1'].child_dev_days, 2);
  assert.equal(out['ABC-1'].first_commit_at, T('2026-06-02T10:00:00Z'));
  assert.deepEqual(out['ABC-1'].timing_source_subtasks, ['ABC-2']);
});

test('overlapping checklist intervals are a union, not a sum', () => {
  const p = story({ intervals: [], dev_days: 0.1 });
  const s1 = sub({ key: 'ABC-2', intervals: [iv('2026-06-01T00:00:00Z', '2026-06-04T00:00:00Z')], dev_days: 3 }); // Mon–Wed
  const s2 = sub({ key: 'ABC-3', intervals: [iv('2026-06-02T00:00:00Z', '2026-06-05T00:00:00Z')], dev_days: 3 }); // Tue–Thu
  const out = by(S.classifySubtasks([p, s1, s2]));
  assert.equal(out['ABC-1'].child_dev_days, 4, 'Mon–Thu = 4 days, not 3 + 3');
});

test('design sub-task feeds the design phase, never the dev rollup', () => {
  const d = sub({ key: 'ABC-4', type: 'Sub-task', summary: 'Technical design', dev_days: 2, intervals: [iv('2026-05-25T00:00:00Z', '2026-05-27T00:00:00Z')] });
  const out = by(S.classifySubtasks([story({ design_days: 0 }), d]));
  assert.equal(out['ABC-4'].design_subtask, true);
  assert.equal(out['ABC-4'].rollup, true);
  assert.equal(out['ABC-1'].child_dev_days, undefined);
  assert.equal(out['ABC-1'].design_days_eff, 2);
});

test('unionIntervals merges touching and nested ranges', () => {
  assert.deepEqual(S.unionIntervals([[5, 7], [1, 3], [3, 4], [2, 2.5]]), [[1, 4], [5, 7]]);
});

test('commit days are LOCAL days in the configured zone', () => {
  // 21:30Z and 23:30Z on 06-02 are 06-03 00:30 / 02:30 in UTC+3 → one local day;
  // with 06-02 10:00Z that is 2 local days in UTC+3, but 1 UTC day.
  const s = sub({ key: 'ABC-5', assignee_id: 'u-b', commit_ts: [T('2026-06-02T10:00:00Z'), T('2026-06-02T21:30:00Z'), T('2026-06-02T23:30:00Z')] });
  const utc = by(S.classifySubtasks([story(), s], { min_commit_days: 2 }));
  assert.equal(utc['ABC-5'].rollup, true);
  const local = by(S.classifySubtasks([story(), s], { min_commit_days: 2, timezone: 'Asia/Jerusalem' }));
  assert.equal(local['ABC-5'].rollup, false);
});

test('creditedSubtasks lists real sub-task work per person; parent excludes it', () => {
  const b = sub({
    key: 'ABC-3', assignee_id: 'u-b', summary: 'Migrate cache', dev_days: 2,
    commit_ts: [T('2026-06-08T10:00:00Z'), T('2026-06-09T10:00:00Z')],
    intervals: [iv('2026-06-08T00:00:00Z', '2026-06-10T00:00:00Z')],
    git_authors: [{ name: 'B', email: 'b@x', commits: 3 }],
  });
  const chk = sub({ key: 'ABC-2', summary: 'Checklist', dev_days: 0.1 });
  const out = S.classifySubtasks([story(), b, chk]);
  const parent = out.find((r) => r.key === 'ABC-1');
  assert.equal(parent.dev_days, 3, "story keeps its own days; B's 2 days are not added");
  assert.equal(parent.child_dev_days, 3, "rollup = story's own Mon–Wed union only; B's Mon–Tue next week never feeds it");
  const c = S.creditedSubtasks(out);
  assert.deepEqual(c, { 'u-b': [{ key: 'ABC-3', parent_key: 'ABC-1', summary: 'Migrate cache', dev_days: 2, commits: 3, reason: 'other_assignee', counted_in_parent: false }] });

  // untimed parent adopting same-person timing is surfaced, flagged as counted once
  const p = story({ dev_days: 0, intervals: [], git_authors: [] });
  const s = sub({ key: 'ABC-6', dev_days: 2, commit_ts: [T('2026-06-02T10:00:00Z')] });
  const c2 = S.creditedSubtasks(S.classifySubtasks([p, s]));
  assert.equal(c2['u-a'][0].reason, 'parent_untimed');
  assert.equal(c2['u-a'][0].counted_in_parent, true);
  assert.deepEqual(S.creditedSubtasks([]), {});
});

test('design vocabulary matches phases.js', () => {
  assert.ok(S.RE_DESIGN.test('Spike: queue options'));
  assert.ok(S.RE_DESIGN.test('Investigate timeouts'));
  assert.ok(!S.RE_DESIGN.test('Write unit tests'));
});

test('design sub-tasks use the shared phases.js vocabulary', () => {
  assert.equal(S.RE_DESIGN, require('../lib/phases.js').RE_DESIGN);
});

test('substantive sub-task under an UNASSIGNED story is credited to the sub-task owner', () => {
  const b = sub({ key: 'ABC-4', assignee_id: 'u-b', dev_days: 2, commit_ts: [T('2026-06-08T10:00:00Z'), T('2026-06-09T10:00:00Z')] });
  const out = S.classifySubtasks([story({ assignee_id: null }), b]);
  const r = by(out)['ABC-4'];
  assert.equal(r.substantive_subtask, true);
  assert.equal(r.credited_to, 'u-b');
  const credit = S.creditedSubtasks(out);
  assert.deepEqual(credit['u-b'].map((c) => c.key), ['ABC-4']);
  assert.equal(credit['u-b'][0].counted_in_parent, false);
});

test('sub-task with story points is not double-counted', () => {
  // Checklist with points rolls up: its points are not scope on their own and
  // the story's own points/dev_days are left untouched.
  const chk = sub({ key: 'ABC-5', points: 3, dev_days: 1, intervals: [iv('2026-06-02T00:00:00Z', '2026-06-03T00:00:00Z')] });
  const out = by(S.classifySubtasks([story({ points: 5 }), chk]));
  assert.equal(out['ABC-5'].rollup, true);
  assert.equal(out['ABC-1'].points, 5);
  assert.equal(out['ABC-1'].dev_days, 3);
  assert.equal(out['ABC-1'].child_dev_days, 3, 'inside the story window → union, not 3 + 1');
  // Standalone (other person) with points: credited once, kept out of the parent rollup.
  const b = sub({ key: 'ABC-6', assignee_id: 'u-b', points: 2, dev_days: 2, commit_ts: [T('2026-06-08T10:00:00Z')], intervals: [iv('2026-06-08T00:00:00Z', '2026-06-10T00:00:00Z')] });
  const out2 = S.classifySubtasks([story({ points: 5 }), b]);
  const m = by(out2);
  assert.equal(m['ABC-6'].rollup, false);
  assert.equal(m['ABC-1'].points, 5);
  assert.equal(m['ABC-1'].child_dev_days, undefined);
  assert.equal(Object.values(S.creditedSubtasks(out2)).flat().filter((c) => c.key === 'ABC-6').length, 1);
});
