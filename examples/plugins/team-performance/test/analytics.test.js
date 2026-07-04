// Unit tests for the pure analytics core (no I/O, no network).
// Run (from the plugin dir): node --test   (zero dependencies — node:test builtins only)
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
    done_git_at: T('2026-06-05T11:00:00Z'),
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
  // Git-primary effective timing: first commit -> merge; eff start = first
  // active (earlier than the first commit here).
  assert.equal(r.timing_source, 'git');
  assert.equal(r.eff_done_at, T('2026-06-05T11:00:00Z'));
  assert.ok(Math.abs(r.impl_days_git - 2.96) < 0.01, `impl_git ${r.impl_days_git}`);
  assert.ok(r.eff_cycle_days > 4 && r.eff_cycle_days < 4.1, `eff cycle ${r.eff_cycle_days}`);
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
  // merged Wed Jun 17 (>2 business days after done Jun 10)
  opts.gitIndex.byKey.set('TP-10', { first_commit_at: null, done_git_at: T('2026-06-17T10:00:00Z'), delivered_at: T('2026-06-17T10:00:00Z') });
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

test('suggestGoals: zero medians never suggest an unsaveable 0 target', () => {
  const dev = { median_cycle_days: 3, median_impl_days: 2, median_design_days: 0, estimate_mape: 0.3, avg_wip: 2 };
  const team = { median_cycle_days: 3, median_impl_days: 2, median_design_days: 0, estimate_mape: 0.25, avg_wip: 1.5 };
  const gs = A.suggestGoals(dev, team);
  assert.ok(!gs.some((g) => g.metric === 'median_design_days'), JSON.stringify(gs));
  assert.ok(gs.every((g) => g.target > 0));
});

test('goalProgress: direction-aware semantics', () => {
  assert.deepEqual(A.goalProgress({ metric: 'median_cycle_days', target: 4 }, { median_cycle_days: 3.5 }), {
    current: 3.5,
    met: true,
    dir: 'down',
  });
  assert.equal(A.goalProgress({ metric: 'median_cycle_days', target: 4 }, {}).met, false);
  // Higher-is-better metrics flip the comparison.
  assert.equal(A.goalProgress({ metric: 'weighted_throughput', target: 10 }, { weighted_done: 12 }).met, true);
  assert.equal(A.goalProgress({ metric: 'efficiency', target: 1.2 }, { efficiency: 1.0 }).met, false);
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

// ---- v2: git-primary derivation, overrides, routine, credit, scope goals ----

test('deriveGit: fixes and deploy-wait segments (deploy starts where fixing ends)', () => {
  const base = rec({ key: 'G1', first_active_at: T('2026-06-01T00:00:00Z') });
  const r = A.deriveGit(base, {
    first_commit_at: T('2026-06-01T00:00:00Z'),
    done_git_at: T('2026-06-03T00:00:00Z'),
    last_fix_at: T('2026-06-05T00:00:00Z'),
    fix_count: 2,
    deployed_at: T('2026-06-10T00:00:00Z'),
    authors: [{ name: 'a', email: 'a@x', commits: 3 }],
  }, { workweek: [1, 2, 3, 4, 5], hasRepos: true });
  assert.equal(r.timing_source, 'git');
  assert.ok(Math.abs(r.impl_days_git - 2) < 1e-9, `impl ${r.impl_days_git}`);
  assert.ok(Math.abs(r.fix_days - 2) < 1e-9, `fix ${r.fix_days}`); // Wed->Fri
  // deploy wait counts from the LAST fix (Fri) to deploy (next Wed) = 3 business days
  assert.ok(Math.abs(r.deploy_wait_days - 3) < 1e-9, `deploy ${r.deploy_wait_days}`);
});

test('deriveGit: merged but Jira still open -> done_by_git_only; unmerged code flagged', () => {
  const open = rec({ key: 'G2', done_at: null, cycle_days: null });
  const merged = A.deriveGit(open, { first_commit_at: 1, done_git_at: T('2026-06-03T00:00:00Z') }, { hasRepos: true });
  assert.ok(A.isDone(merged));
  assert.ok(merged.flags.includes('done_by_git_only'));

  const done = rec({ key: 'G3' });
  const unmerged = A.deriveGit(done, { first_commit_at: T('2026-06-01T00:00:00Z') }, { hasRepos: true });
  assert.ok(unmerged.flags.includes('unmerged_code'));
  assert.equal(unmerged.timing_source, 'jira');
});

test('deriveGit: stale Jira timing without git signal is excluded from stats', () => {
  const stale = A.deriveGit(rec({ key: 'S1', cycle_days: 200 }), undefined, { hasRepos: true, staleDays: 45 });
  assert.ok(stale.flags.includes('stale_timing'));
  assert.ok(A.isExcluded(stale));
  const b = A.baselines([stale, rec({ key: 'S2' }), rec({ key: 'S3' }), rec({ key: 'S4' })]);
  assert.equal(b.completed_n, 3, 'stale record left the baseline');
});

test('overrides: outlier excludes; manual time re-includes at the entered value', () => {
  const out = { ...rec({ key: 'O1', cycle_days: 90 }), outlier: true };
  assert.ok(A.isExcluded(out));
  const manual = { ...out, manual_days: 12 };
  assert.ok(!A.isExcluded(manual));
  const b = A.baselines([manual, rec({ key: 'O2', cycle_days: 4 }), rec({ key: 'O3', cycle_days: 4 })]);
  assert.equal(b.completed_n, 3);
  assert.equal(b.lookup('Story', 3).bucket.total.p75, 12, 'manual value feeds the bucket');
});

test('routine detection: repeated version-bump summaries cluster; AI flag also counts', () => {
  const recs = [];
  for (let i = 0; i < 5; i++) recs.push(rec({ key: `U${i}`, summary: `Upgrade wallet to version 5.0.${i}` }));
  recs.push(rec({ key: 'F1', summary: 'Implement multi-currency withdrawal limits' }));
  const sig = A.routineSignatures(recs);
  assert.ok(A.isRoutine(recs[0], sig, undefined), 'bulk signature');
  assert.ok(!A.isRoutine(recs[5], sig, undefined), 'unique feature is not routine');
  assert.ok(A.isRoutine(recs[5], sig, { days: 1, routine: true }), 'AI flag wins');
});

test('author matcher: display name, reversed name, email, alias', () => {
  const m = A.makeAuthorMatcher({
    'id-1': { name: 'Jane Doe', aliases: ['janed@corp.com', 'jane.doe'] },
    'id-2': { name: 'John Smith', aliases: [] },
  });
  assert.equal(m('Jane Doe', 'x@y'), 'id-1');
  assert.equal(m('Doe Jane', 'x@y'), 'id-1');
  assert.equal(m('someone', 'janed@corp.com'), 'id-1');
  assert.equal(m('jane.doe', ''), 'id-1');
  assert.equal(m('John Smith', ''), 'id-2');
  assert.equal(m('Unknown Person', 'u@u'), null);
});

test('contributorCredits: commit-share split; assignee fallback when nobody matches', () => {
  const m = A.makeAuthorMatcher({ 'id-1': { name: 'Alice', aliases: [] }, 'id-2': { name: 'Bob', aliases: [] } });
  const shared = rec({
    key: 'M1',
    assignee_id: 'id-1',
    git_authors: [
      { name: 'Alice', email: 'a@x', commits: 6 },
      { name: 'Bob', email: 'b@x', commits: 2 },
      { name: 'Stranger', email: 's@x', commits: 4 }, // unmatched -> out of the denominator
    ],
  });
  const credits = A.contributorCredits(shared, m);
  assert.equal(credits.length, 2);
  assert.ok(Math.abs(credits[0].share - 0.75) < 1e-9);
  assert.equal(credits[0].person_id, 'id-1');
  assert.ok(Math.abs(credits[1].share - 0.25) < 1e-9);

  const noCode = rec({ key: 'M2', assignee_id: 'id-2', git_authors: [] });
  assert.deepEqual(A.contributorCredits(noCode, m), [{ person_id: 'id-2', share: 1, commits: 0 }]);
});

test('assigneeStats: weighted throughput uses AI estimates and splits multi-dev credit', () => {
  const m = A.makeAuthorMatcher({ 'u-a': { name: 'Alice', aliases: [] }, 'u-b': { name: 'Bob', aliases: [] } });
  const recs = [
    // Task worth 8 est-days, split 50/50 between Alice (assignee) and Bob.
    rec({
      key: 'W1', assignee_id: 'u-a', assignee_name: 'Alice', cycle_days: 4, eff_cycle_days: 4,
      git_authors: [{ name: 'Alice', email: 'a', commits: 2 }, { name: 'Bob', email: 'b', commits: 2 }],
      flags: ['multi_dev'],
    }),
    rec({ key: 'W2', assignee_id: 'u-a', assignee_name: 'Alice', cycle_days: 2, eff_cycle_days: 2 }),
    rec({ key: 'W3', assignee_id: 'u-b', assignee_name: 'Bob', cycle_days: 2, eff_cycle_days: 2 }),
  ];
  const b = A.baselines(recs);
  const estimates = { W1: { days: 8, routine: false }, W2: { days: 2, routine: true }, W3: { days: 2, routine: false } };
  const stats = A.assigneeStats(recs, b, [1, 2, 3, 4, 5], T('2026-06-30T00:00:00Z'), { estimates, matcher: m });
  const alice = stats.find((s) => s.assignee_id === 'u-a');
  const bob = stats.find((s) => s.assignee_id === 'u-b');
  assert.ok(Math.abs(alice.weighted_done - (4 + 2)) < 1e-9, `alice weighted ${alice.weighted_done}`);
  assert.ok(Math.abs(bob.weighted_done - (4 + 2)) < 1e-9, `bob weighted ${bob.weighted_done}`);
  assert.equal(bob.contributed, 1, 'W1 counted as a contribution for Bob');
  assert.ok(alice.routine_done > 0, 'routine share tracked');
  assert.ok(alice.efficiency > 1, 'estimate/actual ratio computed');
});

test('scopeMetrics + evalScopeGoals: fix rate, deploy lead, direction-aware goals', () => {
  const recs = [
    rec({ key: 'SM1', eff_cycle_days: 4, done_git_at: T('2026-06-03T00:00:00Z'), deployed_at: T('2026-06-05T00:00:00Z'), deploy_wait_days: 2, fix_count: 0, eff_done_at: T('2026-06-03T00:00:00Z') }),
    rec({ key: 'SM2', eff_cycle_days: 6, done_git_at: T('2026-06-10T00:00:00Z'), deployed_at: T('2026-06-12T00:00:00Z'), deploy_wait_days: 2, fix_count: 3, eff_done_at: T('2026-06-10T00:00:00Z') }),
    rec({ key: 'SM3', eff_cycle_days: 2, done_git_at: T('2026-06-11T00:00:00Z'), eff_done_at: T('2026-06-11T00:00:00Z') }),
  ];
  const b = A.baselines(recs);
  const v = A.scopeMetrics(recs, b, [1, 2, 3, 4, 5], T('2026-06-20T00:00:00Z'), {});
  assert.equal(v.median_cycle_days, 4);
  assert.equal(v.median_deploy_lead_days, 2);
  assert.ok(Math.abs(v.fix_rate - 1 / 3) < 0.01, `fix rate ${v.fix_rate}`);
  assert.ok(v.weighted_throughput_wk > 0);
  const evald = A.evalScopeGoals(
    [{ metric: 'median_cycle_days', target: 5 }, { metric: 'weighted_throughput_wk', target: 100 }],
    v,
  );
  assert.equal(evald[0].met, true, 'lower-is-better met');
  assert.equal(evald[1].met, false, 'higher-is-better missed');
});

// ---- v3.1: hierarchy rollup, merge, period, size-weighted pace ---------------

test('enrichHierarchy: dev sub-tasks roll up; design sub-tasks paint the parent design phase', () => {
  const story = rec({ key: 'H-1', type: 'Story', eff_cycle_days: 8, cycle_days: 8 });
  const devSub = rec({ key: 'H-2', type: 'Development Sub-Tasks', subtask: true, parent_key: 'H-1', cycle_days: 1 });
  const designSub = rec({ key: 'H-3', type: 'Design sub-task', subtask: true, parent_key: 'H-1', cycle_days: 2, eff_cycle_days: 2 });
  const qaSub = rec({ key: 'H-4', type: 'QA sub task', subtask: true, parent_key: 'H-1', cycle_days: 1 });
  const orphan = rec({ key: 'H-5', type: 'Development Sub-Tasks', subtask: true, parent_key: 'GONE-1', cycle_days: 1 });
  const out = A.enrichHierarchy([story, devSub, designSub, qaSub, orphan]);
  const byKey = Object.fromEntries(out.map((r) => [r.key, r]));
  assert.equal(byKey['H-2'].rollup, true, 'dev sub-task rolled up');
  assert.ok(A.isExcluded(byKey['H-2']), 'rolled-up records leave the stats');
  assert.equal(byKey['H-4'].rollup, undefined, 'QA sub-task stays a standalone work item');
  assert.equal(byKey['H-5'].rollup, undefined, 'orphan sub-task (parent not in corpus) stays');
  assert.equal(byKey['H-1'].design_days_eff, 3, 'story design = own 1 + design child 2');
});

test('makeCanonical: chains resolve, cycles break, merged stats fold together', () => {
  const canonical = A.makeCanonical({
    old1: { name: 'X (old)', merged_into: 'new1' },
    new1: { name: 'X' },
    a: { name: 'A', merged_into: 'b' },
    b: { name: 'B', merged_into: 'a' }, // cycle — must not hang
  });
  assert.equal(canonical('old1'), 'new1');
  assert.equal(canonical('new1'), 'new1');
  assert.ok(['a', 'b'].includes(canonical('a')));

  const recs = [
    rec({ key: 'M-1', assignee_id: 'old1', assignee_name: 'X (old)', cycle_days: 4, eff_cycle_days: 4 }),
    rec({ key: 'M-2', assignee_id: 'new1', assignee_name: 'X', cycle_days: 2, eff_cycle_days: 2 }),
  ];
  const stats = A.assigneeStats(recs, A.baselines(recs), [1, 2, 3, 4, 5], T('2026-06-30T00:00:00Z'), { canonical });
  assert.equal(stats.length, 1, 'one person after merge');
  assert.equal(stats[0].assignee_id, 'new1');
  assert.equal(stats[0].completed, 2);
});

test('assigneeStats: period window filters samples; monthly buckets fill', () => {
  const recs = [
    rec({ key: 'P-1', assignee_id: 'u-a', assignee_name: 'A', cycle_days: 2, eff_cycle_days: 2, done_at: T('2026-06-10T00:00:00Z'), eff_done_at: T('2026-06-10T00:00:00Z') }),
    rec({ key: 'P-2', assignee_id: 'u-a', assignee_name: 'A', cycle_days: 9, eff_cycle_days: 9, done_at: T('2025-03-10T00:00:00Z'), eff_done_at: T('2025-03-10T00:00:00Z') }),
  ];
  const b = A.baselines(recs);
  const now = T('2026-06-30T00:00:00Z');
  const all = A.assigneeStats(recs, b, [1, 2, 3, 4, 5], now, {});
  assert.equal(all[0].completed, 2);
  const windowed = A.assigneeStats(recs, b, [1, 2, 3, 4, 5], now, { sinceMs: T('2026-01-01T00:00:00Z') });
  assert.equal(windowed[0].completed, 1, 'old completion left the window');
  assert.equal(windowed[0].median_cycle, 2);
  // Monthly: P-1 done this month -> last bucket carries its scope days.
  assert.ok(windowed[0].monthly[11] > 0, `monthly ${JSON.stringify(windowed[0].monthly)}`);
  // Historical window [2025 Q1] via until.
  const q1 = A.assigneeStats(recs, b, [1, 2, 3, 4, 5], now, { sinceMs: T('2025-01-01T00:00:00Z'), untilMs: T('2025-04-01T00:00:00Z') });
  assert.equal(q1[0].completed, 1);
  assert.equal(q1[0].median_cycle, 9);
});

test('pace/efficiency are size-weighted: one big miss outweighs many tiny accurate tasks', () => {
  const mk = (k, est, actual, dev) => rec({ key: k, assignee_id: dev, assignee_name: dev, cycle_days: actual, eff_cycle_days: actual });
  const recs = [];
  const estimates = {};
  // Dev "small": ten 0.5d tasks each done in 0.5d, plus one 5d task done in 12d.
  for (let i = 0; i < 10; i++) { recs.push(mk(`S-${i}`, 0.5, 0.5, 'u-s')); estimates[`S-${i}`] = { days: 0.5 }; }
  recs.push(mk('S-big', 5, 12, 'u-s'));
  estimates['S-big'] = { days: 5 };
  // Dev "ref": accurate on the same volume.
  recs.push(mk('R-1', 10, 10, 'u-r'));
  estimates['R-1'] = { days: 10 };
  recs.push(mk('R-2', 3, 3, 'u-r'));
  estimates['R-2'] = { days: 3 };
  recs.push(mk('R-3', 2, 2, 'u-r'));
  estimates['R-3'] = { days: 2 };
  const b = A.baselines(recs);
  const stats = A.assigneeStats(recs, b, [1, 2, 3, 4, 5], T('2026-06-30T00:00:00Z'), { estimates });
  const s = stats.find((x) => x.assignee_id === 'u-s');
  // Σactual 17 / Σest 10 = 1.7 — the median of per-task ratios would be 1.0.
  assert.ok(s.efficiency < 0.7, `efficiency ${s.efficiency}`);
  assert.ok(s.pace_factor > 1.2, `pace ${s.pace_factor}`);
});
