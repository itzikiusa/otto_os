// A goal run's criteria rows (goal.md §3.5, GL8.1) — workflows/goal/goalCheck.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  activeMs,
  checkpointHistoryByCriterion,
  checksByStep,
  criterionKey,
  criterionRows,
  formatNum,
  formatRatio,
  graphCriteria,
  latestCheck,
  metCount,
  newestCheckpoint,
  newestCheckpointsByStep,
  observedPairs,
  progressPct,
  readsCheckpoints,
  recentTrend,
} from '../src/modules/workflows/goal/goalCheck.ts';
import type { CheckOutput, Criterion, GoalProgress, NodeRunState, WorkflowCheckpoint } from '../src/lib/api/types.ts';

const criteria: Criterion[] = [
  { id: 'tests', text: 'Tests pass', verify: 'command', how: '', command: 'go test ./...' },
  { id: 'docs', text: 'Docs updated', verify: 'agent', how: 'read docs/' },
  { id: 'ui', text: 'Looks right', verify: 'human', how: 'open the page' },
];

function out(met: Record<string, [boolean, string]>): CheckOutput {
  const rows = Object.entries(met).map(([id, [m, evidence]]) => ({ id, text: id, verify: 'agent' as const, met: m, evidence, verified_by: 'agent' }));
  return { criteria: rows, all_met: rows.every((r) => r.met), met: rows.filter((r) => r.met).length, total: rows.length, progress_pct: 0, verdict: 'continue', feedback: '', feedback_md: '', repeat_signature: '', repeated_failures: 0 };
}

function cp(iteration: number, step: number, output: unknown, over: Partial<WorkflowCheckpoint> = {}): WorkflowCheckpoint {
  return { node_id: `loop#${iteration}.${step}`, loop_id: 'loop', iteration, step_index: step, kind: 'check', name: 'Check criteria', status: 'success', attempts: 1, input: null, output, logs: [], updated_at: '', ...over };
}

test('the newest settled check wins: last iteration, last step', () => {
  const a = out({ tests: [false, 'verify command failed (exit 1)'] });
  const b = out({ tests: [true, 'verify command exited 0'] });
  const c = out({ docs: [true, 'x'] });
  assert.equal(latestCheck([], [cp(1, 1, a), cp(2, 1, b), cp(2, 0, c, { kind: 'agent_prompt' })]), b);
  // A running or failed attempt is not a result.
  assert.equal(latestCheck([], [cp(1, 1, a), cp(2, 1, b, { status: 'running' })]), a);
  // A page row without its output still names the attempt to fetch.
  assert.equal(newestCheckpoint([cp(1, 1, a), cp(3, 1, undefined)])?.iteration, 3);
});

test("without checkpoints: a settled loop's history, then a standalone check", () => {
  const b = out({ docs: [true, 'cited docs/x.md'] });
  const loopNode = { node_id: 'loop', status: 'success', logs: [], output: { history: [{ steps: { work: { ok: 1 }, check: out({}) } }, { steps: { work: {}, check: b } }] } } as NodeRunState;
  assert.equal(latestCheck([loopNode]), b);
  const solo = { node_id: 'check', status: 'success', logs: [], output: b } as NodeRunState;
  assert.equal(latestCheck([solo], []), b);
  // An output that only looks similar is not a check result.
  assert.equal(latestCheck([{ ...solo, output: { criteria: [] } }]), null);
  assert.equal(latestCheck([], []), null);
});

test("rows follow the node's criteria with Met, Not met, Verify and not checked yet", () => {
  const rows = criterionRows(criteria, out({
    tests: [false, 'verify command failed (exit 1)\n--- verify output ---\nFAIL TestLogin'],
    docs: [true, 'docs/export.md describes streaming\nSection 3, lines 10-40'],
  }), ['ui']);
  assert.deepEqual(rows.map((r) => [r.id, r.state, r.method]), [['tests', 'not_met', 'command'], ['docs', 'met', 'agent'], ['ui', 'verify', 'human']]);
  assert.equal(rows[0].headline, 'verify command failed (exit 1)');
  assert.equal(rows[0].output, 'FAIL TestLogin');
  assert.equal(rows[1].headline, 'docs/export.md describes streaming');
  assert.equal(rows[1].detail, 'Section 3, lines 10-40');
  assert.equal(criterionRows(criteria, null)[0].state, 'pending');
});

test('the criteria come from a check step inside a loop as well as a top-level check', () => {
  const graph = { nodes: [
    { id: 'start', kind: 'manual_trigger', name: 'Start', x: 0, y: 0, params: {} },
    { id: 'loop', kind: 'loop', name: 'Work toward the goal', x: 0, y: 0, params: { steps: [{ kind: 'agent_prompt', params: {} }, { kind: 'check', params: { criteria: criteria.slice(0, 2) } }] } },
    { id: 'solo', kind: 'check', name: 'Check', x: 0, y: 0, params: { criteria: criteria.slice(2) } },
  ] };
  assert.deepEqual(graphCriteria(graph).map((c) => c.id), ['tests', 'docs', 'ui']);
});

test("with no criteria on the graph, the result's own rows are shown", () => {
  assert.deepEqual(criterionRows([], out({ a: [true, 'e'] })).map((r) => r.state), ['met']);
});

test('a live goal run re-reads its checkpoints as iterations land; a settled run read whole does not (CR1-042)', () => {
  const page = [{ node_id: 'c1', loop_id: 'loop', iteration: 0, step_index: 1, kind: 'check', name: 'Check', status: 'success' }] as unknown as WorkflowCheckpoint[];
  // Opened mid-run: the run read carried iteration 1; /progress keeps that page while 2, 3 … land.
  assert.equal(readsCheckpoints({ status: 'running', checkpoints: page }, false), true);
  assert.equal(readsCheckpoints({ status: 'running' }, false), true);
  // Opened on a settled run: its read is complete.
  assert.equal(readsCheckpoints({ status: 'success', checkpoints: page }, false), false);
  // Read while live, then it settled: once more, for the last iteration's check.
  assert.equal(readsCheckpoints({ status: 'success', checkpoints: page }, true), true);
});

// Seen on a running goal: "Criteria 0 of 0 met" over three rows, two of them
// Met, and "0s active" a minute in — the run's counts lag the check the card reads.
test('the card counts the criteria it lists, and a running loop shows the time its attempts banked', () => {
  const rows = criterionRows(criteria, out({ tests: [true, 'verify command exited 0'], docs: [false, 'no'], ui: [true, 'seen'] }));
  assert.deepEqual(metCount(rows, { criteria_met: 0, criteria_total: 0 }), { met: 2, total: 3 });
  // A criterion parked for a person is not met yet.
  assert.deepEqual(metCount(criterionRows(criteria, out({ tests: [true, 'ok'], ui: [true, 'old'] }), ['ui']), { criteria_met: 2, criteria_total: 3 }), { met: 1, total: 3 });
  // Nothing listed (no criteria on the graph, no check yet): the run's counts.
  assert.deepEqual(metCount([], { criteria_met: 1, criteria_total: 4 }), { met: 1, total: 4 });

  assert.equal(activeMs({ active_ms: 0 }, [cp(1, 0, null, { active_ms: 30_000 }), cp(1, 1, null, { active_ms: 8_000 }), cp(2, 0, null)]), 38_000);
  assert.equal(activeMs({ active_ms: 81_000 }, [cp(1, 0, null, { active_ms: 30_000 })]), 81_000, 'a stopped loop says its own');
  assert.equal(activeMs({ active_ms: 0 }), 0);
});

test('two check steps with the same criterion id stay distinct, each with its own progress and history', () => {
  const reviewCrit: Criterion = {
    id: 'review-passes',
    text: 'A fresh independent review passes',
    verify: 'expr',
    how: '',
    expr: 'steps.review.passed == true',
  };
  const graph = {
    nodes: [
      { id: 'start', kind: 'manual_trigger', name: 'Start', x: 0, y: 0, params: {} },
      {
        id: 'ui',
        kind: 'loop',
        name: 'UI',
        x: 0,
        y: 0,
        params: { steps: [{ kind: 'review_run', name: 'review' }, { kind: 'check', name: 'check', params: { criteria: [reviewCrit] } }] },
      },
      {
        id: 'server',
        kind: 'loop',
        name: 'server',
        x: 0,
        y: 0,
        params: { steps: [{ kind: 'review_run', name: 'review' }, { kind: 'check', name: 'check', params: { criteria: [reviewCrit] } }] },
      },
    ],
  };
  const scoped = graphCriteria(graph);
  assert.deepEqual(
    scoped.map((c) => [c.step_id, c.step_name, c.id]),
    [
      ['ui.1', 'UI', 'review-passes'],
      ['server.1', 'server', 'review-passes'],
    ],
  );

  const checkOut = (met: boolean, score: number, blocker?: string): CheckOutput => ({
    criteria: [
      {
        id: 'review-passes',
        text: reviewCrit.text,
        verify: 'expr',
        met,
        evidence: `steps.review.passed == true is ${met}`,
        verified_by: 'rule',
        progress: { value: score, target: 95, label: blocker, values: { 'steps.review.passed': met, score, threshold: 95 } },
      },
    ],
    all_met: met,
    met: met ? 1 : 0,
    total: 1,
    progress_pct: met ? 100 : 0,
    verdict: met ? 'achieved' : 'continue',
    feedback: '',
    feedback_md: '',
    repeat_signature: '',
    repeated_failures: 0,
  });

  const checkpoints: WorkflowCheckpoint[] = [
    cp(1, 1, checkOut(false, 35, '4 must-fix findings left'), { loop_id: 'ui', node_id: 'ui#1.1' }),
    cp(2, 1, checkOut(false, 58, '3 must-fix findings left'), { loop_id: 'ui', node_id: 'ui#2.1' }),
    cp(3, 1, checkOut(false, 72, '2 must-fix findings left'), { loop_id: 'ui', node_id: 'ui#3.1' }),
    cp(1, 1, checkOut(false, 60, '2 must-fix findings left'), { loop_id: 'server', node_id: 'server#1.1' }),
    cp(2, 1, checkOut(false, 80, '1 must-fix finding left'), { loop_id: 'server', node_id: 'server#2.1' }),
    cp(3, 1, checkOut(true, 96), { loop_id: 'server', node_id: 'server#3.1' }),
  ];

  const newest = newestCheckpointsByStep(checkpoints);
  assert.deepEqual([...newest.keys()], ['ui.1', 'server.1']);
  assert.equal(newest.get('ui.1')?.iteration, 3);
  assert.equal(newest.get('server.1')?.iteration, 3);

  const checks = checksByStep([], checkpoints, scoped);
  const rows = criterionRows(scoped, checks, [], undefined, checkpoints);
  assert.equal(rows.length, 2);
  assert.equal(rows[0].key, criterionKey('ui.1', 'review-passes'));
  assert.equal(rows[1].key, criterionKey('server.1', 'review-passes'));
  assert.equal(rows[0].state, 'not_met');
  assert.equal(rows[1].state, 'met');
  assert.equal(formatRatio(rows[0].progress), '72 / 95');
  assert.equal(formatRatio(rows[1].progress), '96 / 95');
  assert.equal(rows[0].progress?.label, '2 must-fix findings left');
  assert.equal(recentTrend(rows[0].history), '35 → 58 → 72');
  assert.equal(recentTrend(rows[1].history), '60 → 80 → 96');
  assert.deepEqual(metCount(rows, { criteria_met: 0, criteria_total: 2 }), { met: 1, total: 2 });

  const cpHist = checkpointHistoryByCriterion(checkpoints);
  assert.equal(cpHist.get('ui.1::review-passes')?.length, 3);
  assert.equal(cpHist.get('server.1::review-passes')?.length, 3);

  // Overview goal.criteria supplies progress/history even before checkpoints load.
  const goal: Pick<GoalProgress, 'criteria'> = {
    criteria: [
      {
        step_id: 'ui.1',
        step_name: 'UI',
        id: 'review-passes',
        met: false,
        progress: { value: 72, target: 95, label: '3 must-fix findings left' },
        history: [
          { iteration: 1, met: false, value: 35 },
          { iteration: 2, met: false, value: 58 },
          { iteration: 3, met: false, value: 72 },
        ],
      },
      {
        step_id: 'server.1',
        step_name: 'server',
        id: 'review-passes',
        met: true,
        progress: { value: 96, target: 95 },
        history: [{ iteration: 1, met: true, value: 96 }],
      },
    ],
  };
  const fromOverview = criterionRows(scoped, new Map(), [], goal);
  assert.equal(fromOverview[0].state, 'not_met');
  assert.equal(fromOverview[1].state, 'met');
  assert.equal(recentTrend(fromOverview[0].history), '35 → 58 → 72');
});

test('progress formatting helpers: ratios, meters, trends, and observed pairs', () => {
  assert.equal(formatNum(72), '72');
  assert.equal(formatNum(9.5), '9.5');
  assert.equal(formatNum(9.0), '9');
  assert.equal(formatRatio({ value: 72, target: 95 }), '72 / 95');
  assert.equal(formatRatio({ value: 64, target: 80, unit: '%' }), '64% / 80%');
  assert.equal(formatRatio({ value: 120, target: 200, unit: 'ms' }), '120 / 200 ms');
  assert.equal(formatRatio({ label: 'exit 1' }), '');
  assert.equal(progressPct({ value: 72, target: 95 }, 'not_met'), 76);
  assert.equal(progressPct({ value: 96, target: 95 }, 'met'), 100);
  assert.equal(progressPct({ value: 1, target: 0 }, 'not_met'), null);
  // INPUT-1: ceiling and exact-number rules (including flipped numeric equality) never show a directional floor meter.
  assert.equal(progressPct({ value: 300, target: 200 }, 'not_met', 'steps.bench.latency <= 200'), null);
  assert.equal(progressPct({ value: 5, target: 0 }, 'not_met', 'steps.bench.errors == 0'), null);
  assert.equal(progressPct({ value: 300, target: 200 }, 'not_met', '200 >= steps.bench.latency'), null);
  assert.equal(progressPct({ value: 300, target: 200 }, 'not_met', '200 == steps.build.artifact_count'), null);
  // Floor rules (including review score/passed rules) still show a meter.
  assert.equal(progressPct({ value: 72, target: 95 }, 'not_met', 'steps.review.passed == true'), 76);
  assert.equal(progressPct({ value: 64, target: 80 }, 'not_met', 'steps.bench.coverage >= 80'), 80);
  assert.equal(recentTrend([{ iteration: 1, met: false, value: 35 }]), '');
  assert.equal(
    recentTrend([
      { iteration: 1, met: false, value: 20 },
      { iteration: 2, met: false, value: 35 },
      { iteration: 3, met: false, value: 58 },
      { iteration: 4, met: false, value: 72 },
    ]),
    '35 → 58 → 72',
  );
  assert.deepEqual(observedPairs({ values: { 'steps.review.passed': false, score: 72, last_line: 'FAIL' } }), [
    { key: 'steps.review.passed', value: 'false' },
    { key: 'score', value: '72' },
    { key: 'last_line', value: 'FAIL' },
  ]);
});
