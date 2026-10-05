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
