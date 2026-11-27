// The run page's step list (ux.md §4.2, §4.4): execution order, parallel
// groups, a fan-out's children, the phone's done fold and the pane's default
// selection — runs/stepOrder.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { childCrumb, childNoun, childTrail, defaultSelection, doneFold, entrySteps, loopChildren, mergeStepChildren, orderChildren, orderChildTree, orderSteps, RESULT, runtimeChildStatus, settledNodes, stepNumber, timeBar, topoOrder } from '../src/modules/workflows/runs/stepOrder.ts';
import { at, fanoutNodes, node, PR_GRAPH, reviewers, T0 } from './workflowRunFixtures.ts';
import { stepWord } from '../src/modules/workflows/runs/runWords.ts';
import type { RuntimeSession, WorkflowGraph } from '../src/lib/api/types';

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

// Seen on a goal run: the pane's crumb read "Work toward the goal › Reviewer"
// over "Work · iteration 2", and a loop's fold would say "1 reviewer done".
test('a child is called what it is: an iteration, a reviewer, or just its own name', () => {
  const kid = (label: string, id = 't1') => ({ id, label });
  assert.equal(childCrumb('loop', kid('Work · iteration 2')), 'Iteration 2');
  assert.equal(childCrumb('loop', kid('Iteration 12 · Work, Check', 'iter-12')), 'Iteration 12');
  assert.equal(childCrumb('review_run', kid('Correctness · Codex')), 'Reviewer');
  // Rows their own name says: no second word.
  assert.equal(childCrumb('review_run', kid('Summarizer')), null);
  assert.equal(childCrumb('review_run', kid('Scope assessment')), null);
  assert.equal(childCrumb('agent_prompt', kid('claude · earlier turn')), null);
  assert.equal(childCrumb('agent_prompt', kid('Research the API')), null);
  assert.equal(childCrumb(undefined, kid('Correctness')), null);

  assert.equal(childNoun('loop', kid('Work · iteration 1')), 'iteration');
  assert.equal(childNoun('review_run', kid('Security · Claude')), 'reviewer');
  assert.equal(childNoun('agent_prompt', kid('claude · earlier turn')), 'thread');
});

// Seen on a real history: a run a daemon restart interrupted under the old
// engine keeps its step `running`; the failed run's page said "Working 3463h 47m".
test('a settled run shows a step left running as stopped, with no live timer', () => {
  const left = node('n3', 'running', 1, null, { phase: 'working', activity: { phase: 'reading', updated_at: at(1), pending_tasks: 0, subagents: [] } });
  const nodes = [node('n1', 'success', 0, 1000), left];
  const [, stopped] = settledNodes(nodes, false);
  assert.deepEqual([stopped.status, stopped.error_class, stopped.phase, stopped.activity], ['skipped', 'canceled', undefined, undefined]);
  assert.deepEqual(stepWord(stopped, { now: NOW + 3463 * 3_600_000, run: 'failed' }), { word: 'Stopped', tone: '', glyph: 'dash' });
  const step = orderSteps([stopped], PR_GRAPH, NOW + 3463 * 3_600_000).flatMap(entrySteps)[0];
  assert.equal(step.end, step.start, 'its bar does not grow with the clock');
  // Before: the same step on the same settled run.
  assert.match(stepWord(left, { now: NOW + 3463 * 3_600_000, run: 'failed' }).word, /^Working 3463h/);
  // A live run's steps are untouched.
  assert.equal(settledNodes(nodes, true), nodes);
});

// Seen on a real history: every pre-upgrade step's pane was headed "Step – of 6".
test('a step without a stored number takes its place in the run\'s order', () => {
  const steps = orderSteps([node('n1', 'success', 0, 1000), node('n3', 'success', 2, 1000), node('n2', 'success', 1, 1000)], PR_GRAPH, NOW).flatMap(entrySteps);
  assert.deepEqual(steps.map((s) => [s.id, stepNumber(steps, s)]), [['n1', 1], ['n2', 2], ['n3', 3]]);
  // The engine's own number wins.
  const numbered = orderSteps([node('n1', 'success', 0, 1000, { step_no: 4 })], PR_GRAPH, NOW).flatMap(entrySteps);
  assert.equal(stepNumber(numbered, numbered[0]), 4);
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

const rtSession = (id: string, extra: Partial<RuntimeSession> = {}): RuntimeSession => ({
  id,
  workspace_id: 'ws',
  provider: 'codex',
  title: `Thread ${id}`,
  cwd: '/tmp',
  access_mode: 'approval-required',
  capabilities: {} as RuntimeSession['capabilities'],
  status: 'working',
  created_by: 'u1',
  created_at: `2026-11-26T10:00:00Z`,
  last_active_at: `2026-11-26T10:00:00Z`,
  last_sequence: 1,
  ...extra,
});

test('runtimeChildStatus maps live runtime session states into step-list words', () => {
  assert.equal(runtimeChildStatus(rtSession('a', { status: 'working' })), 'running');
  assert.equal(runtimeChildStatus(rtSession('a', { status: 'starting' })), 'running');
  assert.equal(runtimeChildStatus(rtSession('a', { status: 'awaiting_permission' })), 'needs_you');
  assert.equal(runtimeChildStatus(rtSession('a', { status: 'idle', background_tasks: [{ id: 'bg', label: 'cmd', kind: 'command', started_at: at(0) }] })), 'running');
  assert.equal(runtimeChildStatus(rtSession('a', { status: 'idle', last_turn_status: 'completed' })), 'success');
  assert.equal(runtimeChildStatus(rtSession('a', { status: 'idle', last_turn_status: 'failed' })), 'error');
  assert.equal(runtimeChildStatus(rtSession('a', { status: 'idle', last_turn_status: 'interrupted' })), 'skipped');
  assert.equal(runtimeChildStatus(rtSession('a', { status: 'closed' })), 'success');
  assert.equal(runtimeChildStatus(rtSession('a', { status: 'dormant' })), 'skipped');
});

test('mergeStepChildren merges live multi-generation sub-threads with recorded rows', () => {
  const sessions: RuntimeSession[] = [
    rtSession('th-root', { provider: 'claude', title: 'Step thread' }),
    rtSession('th-kid', {
      provider: 'codex',
      title: 'Check tests',
      parent_id: 'th-root',
      status: 'idle',
      last_turn_status: 'completed',
      created_at: '2026-11-26T10:01:00Z',
      meta: { child_of: { id: 'th-root', by: 'agent' } },
    }),
    rtSession('th-grand', {
      provider: 'claude',
      title: 'Inspect helper',
      parent_id: 'th-root',
      status: 'working',
      created_at: '2026-11-26T10:02:00Z',
      meta: { child_of: { id: 'th-kid', by: 'agent' }, provider_slug: 'claude-work' },
    }),
    rtSession('th-other', { provider: 'claude', title: 'Unrelated thread', parent_id: 'th-else' }),
  ];

  const merged = mergeStepChildren([], 'th-root', sessions);
  assert.deepEqual(
    merged.map((c) => [c.id, c.label, c.provider, c.status, c.depth, c.parent_child_id]),
    [
      ['th-kid', 'Check tests', 'codex', 'success', 0, null],
      ['th-grand', 'Inspect helper', 'claude-work', 'running', 1, 'th-kid'],
    ],
  );

  // Ordering keeps the done parent in `active` because its grandchild is still running,
  // and records each nested row's parent label for screen-reader context.
  const tree = orderChildTree(merged);
  assert.deepEqual(
    tree.active.map((e) => [e.child.id, e.depth, e.parentLabel]),
    [['th-kid', 0, null], ['th-grand', 1, 'Check tests']],
  );
  assert.equal(tree.done.length, 0);

  // Breadcrumb trail walks parent_child_id from top child down to the open grandchild.
  assert.deepEqual(childTrail(merged, 'th-grand').map((c) => c.id), ['th-kid', 'th-grand']);
  assert.deepEqual(childTrail(merged, 'th-kid').map((c) => c.id), ['th-kid']);
  assert.deepEqual(childTrail(merged, 'missing'), []);
});

test('mergeStepChildren attaches agent-opened sub-threads underneath a recorded reviewer', () => {
  const recorded = [
    { id: 'lens-1', label: 'Correctness · Codex', provider: 'codex', status: 'running' as const, thread_id: 'th-rev-1' },
    { id: 'summarizer', label: 'Summarizer', provider: 'claude', status: 'queued' as const },
  ];
  const sessions = [
    rtSession('th-rev-1', { provider: 'codex', title: 'Correctness' }),
    rtSession('th-sub', {
      provider: 'claude',
      title: 'Verify edge case',
      parent_id: 'th-rev-1',
      status: 'working',
      meta: { child_of: { id: 'th-rev-1', by: 'agent' } },
    }),
  ];
  const merged = mergeStepChildren(recorded, null, sessions);
  const tree = orderChildTree(merged);
  assert.deepEqual(
    tree.active.map((e) => [e.child.id, e.depth]),
    [['lens-1', 0], ['th-sub', 1]],
  );
  assert.deepEqual(tree.trailing.map((e) => e.child.id), ['summarizer']);
  assert.deepEqual(childTrail(merged, 'th-sub').map((c) => c.id), ['lens-1', 'th-sub']);
});

test('defaultSelection stays on an agent step with its own thread even while a sub-thread runs', () => {
  const stepWithSub = node('n3', 'running', 1, null, {
    thread: { thread_id: 'th-root', runner: 'threads', provider: 'claude', turns: 0, usage: {} as never },
    children: [{ id: 'th-kid', label: 'Check tests', provider: 'codex', status: 'running', thread_id: 'th-kid' }],
  });
  const entries = orderSteps([node('n1', 'success', 0, 0), stepWithSub], PR_GRAPH, NOW);
  assert.deepEqual(defaultSelection('running', entries), { node: 'n3', child: null });
});

test('defaultSelection still hops into a running inner child on a loop step that also has a thread', () => {
  const loopGraph: WorkflowGraph = {
    nodes: [
      { id: 'start', kind: 'manual_trigger', name: 'Start', x: 0, y: 0, params: {} },
      { id: 'goal', kind: 'loop', name: 'Work toward the goal', x: 100, y: 0, params: {} },
    ],
    edges: [{ id: 'e1', source: 'start', target: 'goal' }],
  };
  const loopNode = node('goal', 'running', 1, null, {
    thread: { thread_id: 'th-loop', runner: 'threads', provider: 'claude', turns: 0, usage: {} as never },
    children: [{ id: 'iter-2', label: 'Work · iteration 2', provider: 'claude', status: 'running', thread_id: 'th-iter-2' }],
  });
  const entries = orderSteps([node('start', 'success', 0, 0), loopNode], loopGraph, NOW);
  assert.deepEqual(defaultSelection('running', entries), { node: 'goal', child: 'iter-2' });
});

test('mergeStepChildren deduplicates repeated recorded sub-threads from a same-thread loop', () => {
  const recorded = [
    { id: 'c-first', label: 'Helper', provider: 'codex', status: 'running' as const, thread_id: 'th-helper' },
    { id: 'c-other', label: 'Other', provider: 'claude', status: 'success' as const, thread_id: 'th-other' },
    { id: 'c-second', label: 'Helper', provider: 'codex', status: 'success' as const, thread_id: 'th-helper', ended_at: '2026-11-26T10:05:00Z' },
  ];
  // Deduplicates even with no live runtime sessions loaded yet.
  const offline = mergeStepChildren(recorded, 'th-root', []);
  assert.deepEqual(
    offline.map((c) => [c.id, c.thread_id, c.status, c.ended_at ?? null]),
    [
      ['c-first', 'th-helper', 'success', '2026-11-26T10:05:00Z'],
      ['c-other', 'th-other', 'success', null],
    ],
  );
  // And attaches grandchildren to the canonical deduplicated parent row.
  const sessions = [
    rtSession('th-helper', { provider: 'codex', title: 'Helper', parent_id: 'th-root', status: 'idle', last_turn_status: 'completed' }),
    rtSession('th-grand', {
      provider: 'claude',
      title: 'Deep check',
      parent_id: 'th-root',
      status: 'working',
      meta: { child_of: { id: 'th-helper', by: 'agent' } },
    }),
  ];
  const merged = mergeStepChildren(recorded, 'th-root', sessions);
  assert.deepEqual(
    merged.map((c) => [c.id, c.depth, c.parent_child_id]),
    [
      ['c-first', 0, null],
      ['c-other', 0, null],
      ['th-grand', 1, 'c-first'],
    ],
  );
});
