// Unit tests for the pure analytics core (no I/O, no network).
// Run: node --test test/   (zero dependencies — node:test builtins only)
const { test } = require('node:test');
const assert = require('node:assert/strict');

const A = require('../lib/analytics.js');

// ---- time helpers (all UTC) ------------------------------------------------
// 2026-06-01 is a Monday; June 6/7 = Sat/Sun.
const T = (s) => Date.parse(s);
const DAY = 86400000;

// ---- businessDays ----------------------------------------------------------

test('businessDays: within one workday is fractional', () => {
  const v = A.businessDays(T('2026-06-01T09:00:00Z'), T('2026-06-01T15:00:00Z'));
  assert.ok(Math.abs(v - 0.25) < 1e-9, `got ${v}`);
});

test('businessDays: span across a weekend excludes it', () => {
  // Fri 12:00 -> Mon 12:00 = 0.5 (Fri pm) + 0.5 (Mon am) = 1.0
  const v = A.businessDays(T('2026-06-05T12:00:00Z'), T('2026-06-08T12:00:00Z'));
  assert.ok(Math.abs(v - 1.0) < 1e-9, `got ${v}`);
});

test('businessDays: exact Mon..Sat midnight week = 5.0', () => {
  const v = A.businessDays(T('2026-06-01T00:00:00Z'), T('2026-06-06T00:00:00Z'));
  assert.ok(Math.abs(v - 5.0) < 1e-9, `got ${v}`);
});

test('businessDays: Sun–Thu workweek counts Sunday, excludes Friday', () => {
  const ww = [0, 1, 2, 3, 4];
  // Fri 00:00 -> Mon 00:00: Sun counts = 1.0
  const v = A.businessDays(T('2026-06-05T00:00:00Z'), T('2026-06-08T00:00:00Z'), ww);
  assert.ok(Math.abs(v - 1.0) < 1e-9, `got ${v}`);
});

test('businessDays: zero and negative spans are 0', () => {
  assert.equal(A.businessDays(5, 5), 0);
  assert.equal(A.businessDays(9, 5), 0);
});

// ---- addBusinessDays -------------------------------------------------------

test('addBusinessDays: lands on a workday, skipping the weekend', () => {
  // Thu 12:00 + 2 business days -> Mon 12:00
  const out = A.addBusinessDays(T('2026-06-04T12:00:00Z'), 2);
  assert.equal(new Date(out).toISOString(), '2026-06-08T12:00:00.000Z');
});

test('addBusinessDays: zero days returns start (or next workday from weekend)', () => {
  assert.equal(A.addBusinessDays(T('2026-06-01T09:00:00Z'), 0), T('2026-06-01T09:00:00Z'));
});

// ---- classifyStatus --------------------------------------------------------

test('classifyStatus: defaults', () => {
  assert.equal(A.classifyStatus('In Design', {}), 'design');
  assert.equal(A.classifyStatus('Solution Architecture', {}), 'design');
  assert.equal(A.classifyStatus('In Progress', {}), 'implementation');
  assert.equal(A.classifyStatus('Code Review', {}), 'implementation');
  assert.equal(A.classifyStatus('QA', {}), 'implementation');
  assert.equal(A.classifyStatus('To Do', {}), 'waiting');
  assert.equal(A.classifyStatus('Blocked', {}), 'waiting');
  assert.equal(A.classifyStatus('Done', {}), 'excluded');
  assert.equal(A.classifyStatus('Closed', {}), 'excluded');
});

test('classifyStatus: "Design Review" is design (design wins over review)', () => {
  assert.equal(A.classifyStatus('Design Review', {}), 'design');
});

test('classifyStatus: explicit map overrides defaults (case-insensitive)', () => {
  assert.equal(A.classifyStatus('In Review', { 'in review': 'waiting' }), 'waiting');
  assert.equal(A.classifyStatus('In Review', { 'In Review': 'waiting' }), 'waiting');
});

test('classifyStatus: unknown active-looking status defaults to implementation', () => {
  assert.equal(A.classifyStatus('Doing Stuff 42', {}), 'implementation');
});

// ---- buildIntervals --------------------------------------------------------

function history(created, from, to) {
  return {
    created,
    items: [{ field: 'status', fromString: from, toString: to }],
  };
}

test('buildIntervals: no changelog -> single open interval in created status', () => {
  const iv = A.buildIntervals(T('2026-06-01T09:00:00Z'), { histories: [] }, T('2026-06-02T09:00:00Z'));
  assert.equal(iv.length, 1);
  assert.equal(iv[0].status, 'Unknown');
  assert.equal(iv[0].from, T('2026-06-01T09:00:00Z'));
  assert.equal(iv[0].to, T('2026-06-02T09:00:00Z'));
});

test('buildIntervals: transitions produce contiguous intervals; initial from fromString', () => {
  const cl = {
    histories: [
      history('2026-06-01T10:00:00Z', 'To Do', 'In Progress'),
      history('2026-06-03T10:00:00Z', 'In Progress', 'Done'),
    ],
  };
  const iv = A.buildIntervals(T('2026-06-01T09:00:00Z'), cl, T('2026-06-04T00:00:00Z'));
  assert.deepEqual(
    iv.map((i) => i.status),
    ['To Do', 'In Progress', 'Done'],
  );
  assert.equal(iv[0].from, T('2026-06-01T09:00:00Z'));
  assert.equal(iv[0].to, T('2026-06-01T10:00:00Z'));
  assert.equal(iv[2].to, T('2026-06-04T00:00:00Z'));
});

test('buildIntervals: out-of-order histories are sorted', () => {
  const cl = {
    histories: [
      history('2026-06-03T10:00:00Z', 'In Progress', 'Done'),
      history('2026-06-01T10:00:00Z', 'To Do', 'In Progress'),
    ],
  };
  const iv = A.buildIntervals(T('2026-06-01T09:00:00Z'), cl, T('2026-06-04T00:00:00Z'));
  assert.deepEqual(
    iv.map((i) => i.status),
    ['To Do', 'In Progress', 'Done'],
  );
});

// ---- analyzeIssue ----------------------------------------------------------

function rawIssue({
  key = 'TP-1',
  type = 'Story',
  status = 'Done',
  category = 'done',
  assignee = { accountId: 'u-alice', displayName: 'Alice' },
  created = '2026-06-01T09:00:00Z',
  resolutiondate = null,
  points = 3,
  timeoriginalestimate = null,
  histories = [],
} = {}) {
  return {
    key,
    fields: {
      summary: `${key} summary`,
      issuetype: { name: type },
      status: { name: status, statusCategory: { key: category } },
      assignee,
      created,
      resolutiondate,
      customfield_10016: points,
      timeoriginalestimate,
      updated: '2026-06-30T00:00:00Z',
    },
    changelog: { histories },
  };
}

const OPTS = () => ({
  statusMap: {},
  workweek: [1, 2, 3, 4, 5],
  pointsField: 'customfield_10016',
  gitIndex: { byKey: new Map(), hasRepos: true },
  hasDesignStatuses: true,
  nowMs: T('2026-06-30T00:00:00Z'),
});

test('analyzeIssue: completed story numbers are exact', () => {
  const raw = rawIssue({
    resolutiondate: '2026-06-05T10:00:00Z',
    histories: [
      history('2026-06-01T10:00:00Z', 'To Do', 'In Design'),
      history('2026-06-02T10:00:00Z', 'In Design', 'In Progress'),
      history('2026-06-04T10:00:00Z', 'In Progress', 'In Review'),
      history('2026-06-05T10:00:00Z', 'In Review', 'Done'),
    ],
  });
  const opts = OPTS();
  opts.gitIndex.byKey.set('TP-1', {
    first_commit_at: T('2026-06-02T12:00:00Z'),
    delivered_at: T('2026-06-05T11:00:00Z'),
  });
  const r = A.analyzeIssue(raw, opts);
  assert.equal(r.key, 'TP-1');
  assert.equal(r.assignee_id, 'u-alice');
  assert.equal(r.points, 3);
  assert.equal(r.done_at, T('2026-06-05T10:00:00Z'));
  assert.ok(Math.abs(r.design_days - 1.0) < 1e-9, `design ${r.design_days}`);
  assert.ok(Math.abs(r.impl_days - 3.0) < 1e-9, `impl ${r.impl_days}`);
  // cycle: first active (In Design, Jun 1 10:00) -> done Jun 5 10:00 = 4.0
  assert.ok(Math.abs(r.cycle_days - 4.0) < 1e-9, `cycle ${r.cycle_days}`);
  assert.equal(r.delivered_at, T('2026-06-05T11:00:00Z'));
  assert.deepEqual(r.flags, []);
});

test('analyzeIssue: flags no_code / no_estimate / skipped_design', () => {
  const raw = rawIssue({
    key: 'TP-9',
    points: null,
    resolutiondate: '2026-06-03T10:00:00Z',
    histories: [
      history('2026-06-01T10:00:00Z', 'To Do', 'In Progress'),
      history('2026-06-03T10:00:00Z', 'In Progress', 'Done'),
    ],
  });
  const r = A.analyzeIssue(raw, OPTS());
  assert.ok(r.flags.includes('no_code'), r.flags.join(','));
  assert.ok(r.flags.includes('no_estimate'));
  assert.ok(r.flags.includes('skipped_design'));
});

test('analyzeIssue: reopened + late_merge flags', () => {
  const raw = rawIssue({
    key: 'TP-10',
    resolutiondate: '2026-06-10T10:00:00Z',
    histories: [
      history('2026-06-01T10:00:00Z', 'To Do', 'In Progress'),
      history('2026-06-03T10:00:00Z', 'In Progress', 'Done'),
      history('2026-06-04T10:00:00Z', 'Done', 'In Progress'), // reopened
      history('2026-06-10T10:00:00Z', 'In Progress', 'Done'),
    ],
  });
  const opts = OPTS();
  // delivered Wed Jun 17 (>2 business days after done Jun 10)
  opts.gitIndex.byKey.set('TP-10', { first_commit_at: null, delivered_at: T('2026-06-17T10:00:00Z') });
  const r = A.analyzeIssue(raw, opts);
  assert.ok(r.flags.includes('reopened'));
  assert.ok(r.flags.includes('late_merge'));
});

test('analyzeIssue: attribution goes to the assignee at done, not the current one', () => {
  const raw = rawIssue({
    key: 'TP-11',
    assignee: { accountId: 'u-carol', displayName: 'Carol' }, // reassigned AFTER done
    resolutiondate: '2026-06-05T10:00:00Z',
    histories: [
      history('2026-06-01T10:00:00Z', 'To Do', 'In Progress'),
      history('2026-06-05T10:00:00Z', 'In Progress', 'Done'),
      {
        created: '2026-06-09T10:00:00Z',
        items: [{ field: 'assignee', from: 'u-alice', to: 'u-carol', fromString: 'Alice', toString: 'Carol' }],
      },
    ],
  });
  const r = A.analyzeIssue(raw, OPTS());
  assert.equal(r.assignee_id, 'u-alice');
  assert.equal(r.assignee_name, 'Alice');
});

test('analyzeIssue: done_at falls back to last done-transition without resolutiondate', () => {
  const raw = rawIssue({
    key: 'TP-12',
    resolutiondate: null,
    histories: [
      history('2026-06-01T10:00:00Z', 'To Do', 'In Progress'),
      history('2026-06-03T10:00:00Z', 'In Progress', 'Done'),
    ],
  });
  const r = A.analyzeIssue(raw, OPTS());
  assert.equal(r.done_at, T('2026-06-03T10:00:00Z'));
});

test('analyzeIssue: open issue has null done/cycle and open intervals', () => {
  const raw = rawIssue({
    key: 'TP-13',
    status: 'In Progress',
    category: 'indeterminate',
    histories: [history('2026-06-22T10:00:00Z', 'To Do', 'In Progress')],
    created: '2026-06-20T09:00:00Z',
  });
  const r = A.analyzeIssue(raw, OPTS());
  assert.equal(r.done_at, null);
  assert.equal(r.cycle_days, null);
  assert.ok(r.impl_days > 0);
});

// ---- baselines / verdict / factor / predict --------------------------------

function rec(over) {
  return {
    key: 'K',
    type: 'Story',
    points: 3,
    assignee_id: 'u-x',
    design_days: 1,
    impl_days: 3,
    cycle_days: 4,
    done_at: T('2026-06-05T10:00:00Z'),
    ...over,
  };
}

test('baselines: bucket stats + fallback chain + min-n', () => {
  const recs = [
    rec({ key: 'A1', cycle_days: 2 }),
    rec({ key: 'A2', cycle_days: 4 }),
    rec({ key: 'A3', cycle_days: 6 }),
    rec({ key: 'B1', type: 'Bug', points: null, cycle_days: 10 }), // lone bug
  ];
  const b = A.baselines(recs);
  const hit = b.lookup('Story', 3);
  assert.equal(hit.level, 'type+points');
  assert.equal(hit.bucket.n, 3);
  assert.equal(hit.bucket.total.p50, 4);
  // lone bug bucket has n=1 -> falls back to type ('Bug' n=1) -> all (n=4)
  const fb = b.lookup('Bug', null);
  assert.equal(fb.level, 'all');
  assert.equal(fb.bucket.n, 4);
  // unknown type falls back to all
  assert.equal(b.lookup('Epic', 8).level, 'all');
});

test('baselines: too-small corpus -> null lookup', () => {
  const b = A.baselines([rec({}), rec({ key: 'K2' })]);
  assert.equal(b.lookup('Story', 3), null);
});

test('verdict boundaries: 0.8 and 1.25 are inclusive on-track edges', () => {
  assert.equal(A.verdict(0.8, 1), 'fast');
  assert.equal(A.verdict(0.81, 1), 'on_track');
  assert.equal(A.verdict(1.25, 1), 'on_track');
  assert.equal(A.verdict(1.26, 1), 'slow');
  assert.equal(A.verdict(null, 1), null);
  assert.equal(A.verdict(2, null), null);
  assert.equal(A.verdict(0, 0), 'on_track');
  assert.equal(A.verdict(1, 0), null);
});

test('assigneeFactor: median ratio, clamped, needs 3 samples', () => {
  const recs = [];
  for (let i = 0; i < 6; i++) {
    recs.push(rec({ key: `T${i}`, assignee_id: 'u-team', cycle_days: 4 }));
  }
  // fast dev: 3 tasks at half the team p50
  for (let i = 0; i < 3; i++) {
    recs.push(rec({ key: `F${i}`, assignee_id: 'u-fast', cycle_days: 1 }));
  }
  // dev with too few samples
  recs.push(rec({ key: 'S1', assignee_id: 'u-few', cycle_days: 40 }));
  const b = A.baselines(recs);
  const f = A.assigneeFactor(recs, b);
  assert.ok(f.get('u-fast').factor <= 0.5 + 1e-9, `fast ${f.get('u-fast').factor}`); // clamped at 0.5
  assert.equal(f.get('u-few') ? f.get('u-few').factor : 1.0, 1.0);
});

test('predict: open task gets scaled range; in-progress projects a workday finish', () => {
  const recs = [
    rec({ key: 'C1', cycle_days: 2, design_days: 0.5, impl_days: 1.5 }),
    rec({ key: 'C2', cycle_days: 4, design_days: 1, impl_days: 3 }),
    rec({ key: 'C3', cycle_days: 6, design_days: 1.5, impl_days: 4.5 }),
  ];
  const b = A.baselines(recs);
  const open = {
    key: 'O1',
    type: 'Story',
    points: 3,
    assignee_id: null,
    design_days: 1,
    impl_days: 1,
    done_at: null,
  };
  // now = Friday 12:00
  const now = T('2026-06-05T12:00:00Z');
  const p = A.predict(open, b, new Map(), now, [1, 2, 3, 4, 5]);
  assert.equal(p.total.p50, 4);
  assert.ok(Math.abs(p.elapsed_active_days - 2) < 1e-9);
  assert.ok(Math.abs(p.pct_consumed - 0.5) < 1e-9);
  // remaining 2 business days from Fri 12:00 -> Tue 12:00
  assert.equal(new Date(p.projected_done_at).toISOString(), '2026-06-09T12:00:00.000Z');
  const dow = new Date(p.projected_done_at).getUTCDay();
  assert.ok(dow >= 1 && dow <= 5);
});

test('predict: no baseline -> null', () => {
  const b = A.baselines([]);
  assert.equal(A.predict(rec({ done_at: null }), b, new Map(), T('2026-06-05T12:00:00Z'), [1, 2, 3, 4, 5]), null);
});

// ---- goals -----------------------------------------------------------------

test('suggestGoals: slow dev steps 10% toward team p50; fast dev keeps own median', () => {
  const slow = { median_cycle_days: 6, median_impl_days: 5, median_design_days: 2, estimate_mape: 0.5, avg_wip: 3 };
  const team = { median_cycle_days: 3, median_impl_days: 2.5, median_design_days: 1, estimate_mape: 0.25, avg_wip: 1.5 };
  const gs = A.suggestGoals(slow, team);
  const cycle = gs.find((g) => g.metric === 'median_cycle_days');
  assert.ok(Math.abs(cycle.target - 5.4) < 1e-9, `got ${cycle.target}`); // 6*0.9=5.4 > team 3

  const fast = { median_cycle_days: 2, median_impl_days: 1, median_design_days: 0.5, estimate_mape: 0.1, avg_wip: 1 };
  const gf = A.suggestGoals(fast, team);
  const fcycle = gf.find((g) => g.metric === 'median_cycle_days');
  assert.equal(fcycle.target, 2); // never suggest regression (team p50=3 would be worse)
});

test('goalProgress: lower-is-better semantics', () => {
  assert.deepEqual(A.goalProgress({ metric: 'median_cycle_days', target: 4 }, { median_cycle_days: 3.5 }), {
    current: 3.5,
    met: true,
  });
  assert.equal(A.goalProgress({ metric: 'median_cycle_days', target: 4 }, {}).met, false);
});

// ---- assigneeStats ----------------------------------------------------------

test('assigneeStats: medians, mape, wip and trend', () => {
  const recs = [];
  // 6 completed for alice with improving cycles (older first by done_at)
  const cycles = [8, 8, 8, 4, 4, 4];
  cycles.forEach((c, i) => {
    recs.push(
      rec({
        key: `A${i}`,
        assignee_id: 'u-alice',
        assignee_name: 'Alice',
        cycle_days: c,
        estimate_days: 4,
        done_at: T('2026-06-01T00:00:00Z') + i * DAY,
      }),
    );
  });
  // one open task, active
  recs.push(
    rec({
      key: 'A-open',
      assignee_id: 'u-alice',
      assignee_name: 'Alice',
      done_at: null,
      cycle_days: null,
      first_active_at: T('2026-06-20T00:00:00Z'),
    }),
  );
  const b = A.baselines(recs);
  const stats = A.assigneeStats(recs, b, [1, 2, 3, 4, 5], T('2026-06-30T00:00:00Z'));
  const alice = stats.find((s) => s.assignee_id === 'u-alice');
  assert.equal(alice.completed, 6);
  assert.equal(alice.wip, 1);
  assert.ok(Math.abs(alice.median_cycle - 6) < 1e-9, `median ${alice.median_cycle}`);
  assert.equal(alice.trend, 'improving');
  assert.ok(alice.mape !== null);
});
