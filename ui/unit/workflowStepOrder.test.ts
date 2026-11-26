// The run page's step list (ux.md §4.2, §4.4): execution order, parallel
// groups, a fan-out's children, the phone's done fold and the pane's default
// selection — runs/stepOrder.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { defaultSelection, doneFold, entrySteps, loopChildren, orderChildren, orderSteps, RESULT, timeBar, topoOrder } from '../src/modules/workflows/runs/stepOrder.ts';
import { at, fanoutNodes, node, PR_GRAPH, reviewers, T0 } from './workflowRunFixtures.ts';
import type { WorkflowGraph } from '../src/lib/api/types';

const NOW = T0 + 12 * 60_000;
const shape = (entries: ReturnType<typeof orderSteps>) => entries.map((e) => (e.type === 'group' ? e.steps.map((s) => s.id) : e.step.id));

test('steps follow execution order, not graph order', () => {
  // The graph lists a, b, c, d; the edges run a → d → b → c.
  const g: WorkflowGraph = {
    nodes: ['a', 'b', 'c', 'd'].map((id) => ({ id, kind: 'agent_prompt', name: id, x: 0, y: 0, params: {} })),
    edges: [{ id: '1', source: 'a', target: 'd' }, { id: '2', source: 'd', target: 'b' }, { id: '3', source: 'b', target: 'c' }],
  };
  assert.deepEqual(topoOrder(g), ['a', 'd', 'b', 'c']);
  const nodes = [node('a', 'success', 0, 1000), node('b', 'success', 2, 1000), node('c', 'pending', null, null), node('d', 'success', 1, 1000)];
  assert.deepEqual(shape(orderSteps(nodes, g, NOW)), ['a', 'd', 'b', 'c']);
});

test('overlapping siblings form one group; the skipped sibling joins it', () => {
  const entries = orderSteps(fanoutNodes(), PR_GRAPH, NOW);
  assert.deepEqual(shape(entries), ['n1', 'n2', 'n3', ['n4', 'n5', 'n6'], 'n7', 'n8', 'n9', 'n10']);
});

test('an ancestor and its descendant never group, even when their clocks overlap', () => {
  // n3 "ends" a second after n4 starts (rounded timestamps).
  const nodes = [node('n3', 'success', 0, 50_000), node('n4', 'success', 0.8, 1000)];
  assert.deepEqual(shape(orderSteps(nodes, PR_GRAPH, NOW)), ['n3', 'n4']);
});

test('a pending sibling joins the group of a running step', () => {
  const nodes = [node('n3', 'success', 0, 1000), node('n4', 'running', 1, null), node('n5', 'pending', null, null), node('n7', 'pending', null, null)];
  assert.deepEqual(shape(orderSteps(nodes, PR_GRAPH, NOW)), ['n3', ['n4', 'n5'], 'n7']);
});

test('a run without overlap shows no group (old serial runs)', () => {
  const nodes = [node('n3', 'success', 0, 1000), node('n4', 'success', 1, 1000), node('n5', 'success', 2, 1000), node('n7', 'success', 3, 1000)];
  assert.ok(orderSteps(nodes, PR_GRAPH, NOW).every((e) => e.type === 'step'));
});

test('children: working first, done folded with their findings, summarizer last', () => {
  const v = orderChildren(reviewers(9, 3, 2));
  assert.deepEqual(v.active.map((c) => c.id), ['r0', 'r1', 'r2']);
  assert.deepEqual(v.queued.map((c) => c.id), ['q0', 'q1']);
  assert.equal(v.done.length, 9);
  assert.equal(v.findings, 45); // 1 + 2 + … + 9
  assert.deepEqual(v.trailing.map((c) => c.id), ['summarizer']);
});

test('the phone folds the done steps before the current one', () => {
  const f = doneFold(orderSteps(fanoutNodes(), PR_GRAPH, NOW));
  assert.deepEqual(f, { entries: 4, done: 5, skipped: 1, ms: 0.8 * 60_000 + 192_000 });
  // Nothing to fold before the first step, or when every step is done.
  assert.equal(doneFold(orderSteps([node('n1', 'running', 0, null)], PR_GRAPH, NOW)), null);
});

test('the pane opens on the step that matters for each run state', () => {
  const entries = orderSteps(fanoutNodes(), PR_GRAPH, NOW);
  assert.deepEqual(defaultSelection('running', entries), { node: 'n7', child: 'r0' });
  assert.deepEqual(defaultSelection('succeeded', entries), { node: RESULT, child: null });
  assert.deepEqual(defaultSelection('canceled', entries), { node: RESULT, child: null });
  assert.deepEqual(defaultSelection('queued', entries), { node: 'n1', child: null });
  const failed = orderSteps([node('n1', 'success', 0, 0), node('n7', 'error', 1, 5000), node('n8', 'pending', null, null)], PR_GRAPH, NOW);
  assert.deepEqual(defaultSelection('failed', failed, 'n7'), { node: 'n7', child: null });
  const waits = orderSteps([node('n1', 'success', 0, 0), node('n9', 'running', 1, null, { phase: 'needs_you', attention: { kind: 'approval', text: 'Approve', since: at(2) } })], PR_GRAPH, NOW);
  assert.deepEqual(defaultSelection('needs_you', waits), { node: 'n9', child: null });
  const machine = orderSteps([node('n1', 'success', 0, 0), node('n3', 'running', 1, null, { phase: 'waiting' })], PR_GRAPH, NOW);
  assert.deepEqual(defaultSelection('waiting', machine), { node: 'n3', child: null });
  assert.equal(entrySteps(entries[3]).length, 3);
});

test('a loop step\'s iterations become child rows, newest first, worded from their steps', () => {
  const cp = (iteration: number, step_index: number, name: string, status: string, extra: Record<string, unknown> = {}) =>
    ({ node_id: `work-${iteration}-${step_index}`, loop_id: 'loop', iteration, step_index, kind: 'agent_prompt', name, status, attempts: 1, input: null, output: null, logs: [], updated_at: `2026-11-26T10:0${iteration}:0${step_index}Z`, ...extra }) as never;
  const kids = loopChildren([
    cp(1, 1, 'Check', 'success'), cp(1, 0, 'Work', 'success', { thread_id: 'th-1' }),
    cp(2, 0, 'Work', 'error', { thread_id: 'th-2' }), cp(2, 1, 'Check', 'pending'),
    cp(3, 0, 'Work', 'running', { thread_id: 'th-3' }),
    cp(1, 0, 'Other loop', 'success', { loop_id: 'other' }),
  ], 'loop');
  assert.deepEqual(kids.map((k) => [k.id, k.label, k.status, k.thread_id]), [
    ['iter-3', 'Iteration 3 · Work', 'running', 'th-3'],
    ['iter-2', 'Iteration 2 · Work, Check', 'error', 'th-2'],
    ['iter-1', 'Iteration 1 · Work, Check', 'success', 'th-1'],
  ]);
  assert.equal(kids[0].ended_at, undefined, 'a running iteration has no end');
  assert.equal(kids[2].ended_at, '2026-11-26T10:01:01Z');
});

test('a step\'s time bar stays inside the run\'s clock (CR1-062)', () => {
  const clock = { start: 1_000, end: 11_000 };
  const s = (start: number | null, end: number | null) => ({ id: 'a', name: 'a', kind: '', node: node('a', 'success', 0, 1), start, end });
  assert.deepEqual(timeBar(s(3_500, 6_000), clock), { at: 25, w: 25 });
  assert.equal(timeBar(s(null, null), clock), null);
  // A retry ran past the clock's end, and a skewed start before its beginning.
  for (const b of [timeBar(s(9_000, 40_000), clock)!, timeBar(s(-5_000, 4_000), clock)!, timeBar(s(12_000, 13_000), clock)!]) {
    assert.ok(b.at >= 0 && b.at + b.w <= 100 + 1e-9, JSON.stringify(b));
    assert.ok(b.w > 0, 'still a visible sliver');
  }
});
