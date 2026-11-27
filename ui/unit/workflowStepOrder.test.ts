// The run page's step list (ux.md §4.2, §4.4): execution order, parallel
// groups, a fan-out's children, the phone's done fold and the pane's default
// selection — runs/stepOrder.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  annotateRetriedChildren,
  childCrumb,
  childNoun,
  childTrail,
  defaultSelection,
  doneFold,
  entrySteps,
  groupLoopRounds,
  isWaitingOnSubagents,
  loopChildren,
  loopPositionLine,
  mergeStepChildren,
  orderChildren,
  orderChildTree,
  orderSteps,
  resolveThreadSelection,
  RESULT,
  retriedAttemptReason,
  reuseMergedNode,
  runtimeChildStatus,
  sameNodeFields,
  sameStepChildren,
  settledNodes,
  stepNumber,
  timeBar,
  topoOrder,
  waitingReviewers,
  waitingThreads,
} from '../src/modules/workflows/runs/stepOrder.ts';
import { applyLiveNode, mergeRunProgress } from '../src/modules/workflows/runProgress.ts';
import { paneTabs } from '../src/modules/workflows/runs/stepTabs.ts';
import { at, fanoutNodes, node, PR_GRAPH, reviewers, T0 } from './workflowRunFixtures.ts';
import { childWord, stepWord } from '../src/modules/workflows/runs/runWords.ts';
import type { RuntimeSession, WorkflowGraph, WorkflowRun } from '../src/lib/api/types';

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

test('goal-loop helper keeps its iteration row as parent and Open parent selects the iteration conversation', () => {
  // The daemon mirrors the active iteration's thread id on both `node.thread` and the iteration child row.
  const recorded = [
    { id: 'iter-2', label: 'Work · iteration 2', provider: 'claude', status: 'running' as const, thread_id: 'th-iter-2' },
  ];
  const sessions = [
    rtSession('th-iter-2', { provider: 'claude', title: 'Work · iteration 2' }),
    rtSession('th-helper', {
      provider: 'codex',
      title: 'Check build',
      parent_id: 'th-iter-2',
      status: 'working',
      meta: { child_of: { id: 'th-iter-2', by: 'agent' } },
    }),
  ];
  const merged = mergeStepChildren(recorded, 'th-iter-2', sessions);
  assert.deepEqual(
    merged.map((c) => [c.id, c.depth, c.parent_child_id]),
    [
      ['iter-2', 0, null],
      ['th-helper', 1, 'iter-2'],
    ],
  );
  assert.deepEqual(childTrail(merged, 'th-helper').map((c) => c.id), ['iter-2', 'th-helper']);
  const tree = orderChildTree(merged);
  assert.deepEqual(
    tree.active.map((e) => [e.child.id, e.depth, e.parentLabel]),
    [['iter-2', 0, null], ['th-helper', 1, 'Work · iteration 2']],
  );
  // Clicking "Open parent" inside the helper passes `th-iter-2` -> resolves to `iter-2`, opening Conversation (not Criteria).
  const target = resolveThreadSelection(merged, 'th-iter-2', 'th-iter-2');
  assert.equal(target, 'iter-2');
  assert.equal(paneTabs({ goal: true, child: !!target, kind: 'loop' })[0], 'conversation');
  // Ordinary agent step whose root thread is NOT a child row still resolves back to `null` (the step root).
  assert.equal(resolveThreadSelection([], 'th-step', 'th-step'), null);
});

test('orderSteps without now keeps running steps clock-independent and timeBar resolves them from clock.end', () => {
  const nodes = [node('n3', 'success', 0, 1000), node('n4', 'running', 1, null), node('n5', 'running', 2, null)];
  const entries = orderSteps(nodes, PR_GRAPH);
  assert.deepEqual(shape(entries), ['n3', ['n4', 'n5']]);
  const running = entries.flatMap(entrySteps).find((s) => s.id === 'n4')!;
  assert.equal(running.end, Number.POSITIVE_INFINITY);
  const clock = { start: T0, end: T0 + 10 * 60_000 };
  const bar = timeBar(running, clock)!;
  assert.ok(bar.at > 0 && bar.w > 0 && bar.at + bar.w <= 100 + 1e-9, JSON.stringify(bar));
});

test('sameStepChildren matches structurally identical child arrays and detects status or nesting changes', () => {
  const base = [
    { id: 'c1', label: 'Helper', provider: 'codex', status: 'running' as const, thread_id: 'th-1', depth: 0, parent_child_id: null },
  ];
  const clone = base.map((c) => ({ ...c }));
  assert.equal(sameStepChildren(base, clone), true);
  assert.equal(sameStepChildren(base, [{ ...base[0], status: 'success' as const }]), false);
  assert.equal(sameStepChildren(base, [{ ...base[0], depth: 1 }]), false);
  assert.equal(sameStepChildren(base, []), false);
});

test('annotateRetriedChildren marks earlier failed attempts as muted retried rows in the done fold with log reasons', () => {
  const cp = {
    node_id: 'loop#1.0',
    loop_id: 'loop',
    iteration: 1,
    step_index: 0,
    kind: 'agent_prompt',
    name: 'fix',
    status: 'success',
    attempts: 2,
    input: null,
    output: null,
    logs: [
      'attempt 1: upstream: background work was still running at the limit',
      '✓ checkpoint saved after attempt 2',
    ],
    updated_at: '2026-11-26T10:05:00Z',
  } as never;

  const children = [
    { id: 'th-1', label: 'fix · iteration 1', provider: 'claude', status: 'error' as const, thread_id: 'th-1', depth: 0, parent_child_id: null },
    { id: 'th-2', label: 'fix · iteration 1', provider: 'claude', status: 'running' as const, thread_id: 'th-2', depth: 0, parent_child_id: null },
  ];

  const annotated = annotateRetriedChildren(children, { loopId: 'loop', checkpoints: [cp] });
  assert.equal(annotated[0].retried_attempt, 1);
  assert.equal(annotated[0].retried_reason, 'upstream: background work was still running at the limit');
  assert.equal(annotated[1].retried_attempt, undefined);
  assert.equal(annotated[1].retried_reason, undefined);

  // Word is muted ("Attempt 1 failed · retried"), never danger "Failed".
  assert.deepEqual(childWord(annotated[0], NOW), { word: 'Attempt 1 failed · retried', tone: '' });
  assert.deepEqual(childWord(annotated[1], NOW), { word: 'Working', tone: '' });

  // Order puts the superseded failed attempt into `done` (the "N threads done" fold), leaving only the live attempt in `active`.
  const tree = orderChildTree(annotated);
  assert.deepEqual(tree.active.map((e) => e.child.id), ['th-2']);
  assert.deepEqual(tree.done.map((e) => e.child.id), ['th-1']);
  const flat = orderChildren(annotated);
  assert.deepEqual(flat.active.map((c) => c.id), ['th-2']);
  assert.deepEqual(flat.done.map((c) => c.id), ['th-1']);

  // Last attempt still keeps danger "Failed" and stays in `active` when every retry failed.
  const allFailed = annotateRetriedChildren(
    [
      { id: 'a1', label: 'fix · iteration 1', provider: 'claude', status: 'error' as const, thread_id: 'a1', depth: 0, parent_child_id: null },
      { id: 'a2', label: 'fix · iteration 1', provider: 'claude', status: 'error' as const, thread_id: 'a2', depth: 0, parent_child_id: null },
      { id: 'a3', label: 'fix · iteration 1', provider: 'claude', status: 'error' as const, thread_id: 'a3', depth: 0, parent_child_id: null },
    ],
    {
      nodeLogs: [
        '  [1/fix] attempt 1: provider overloaded',
        '  [1/fix] attempt 2: timed out waiting for tool',
      ],
    },
  );
  assert.deepEqual(childWord(allFailed[0], NOW), { word: 'Attempt 1 failed · retried', tone: '' });
  assert.equal(allFailed[0].retried_reason, 'provider overloaded');
  assert.deepEqual(childWord(allFailed[1], NOW), { word: 'Attempt 2 failed · retried', tone: '' });
  assert.equal(allFailed[1].retried_reason, 'timed out waiting for tool');
  assert.deepEqual(childWord(allFailed[2], NOW), { word: 'Failed', tone: 'danger' });
  const allFailedTree = orderChildTree(allFailed);
  assert.deepEqual(allFailedTree.active.map((e) => e.child.id), ['a3']);
  assert.deepEqual(allFailedTree.done.map((e) => e.child.id), ['a1', 'a2']);

  // Bare node log fallback works for non-loop steps, and missing logs leave reason null cleanly.
  assert.equal(retriedAttemptReason('Build', 1, { nodeLogs: ['attempt 1: exit code 1'] }), 'exit code 1');
  assert.equal(retriedAttemptReason('Build', 2, { nodeLogs: ['attempt 1: exit code 1'] }), null);
  assert.equal(sameStepChildren(annotated, children), false);
});

test('groupLoopRounds groups a review-fix loop by round, nests each review’s reviewers & summarizer, and summarizes earlier rounds', () => {
  const loopGraphNode = {
    id: 'ui',
    kind: 'loop',
    name: 'UI',
    x: 0,
    y: 0,
    params: {
      max_iterations: 5,
      steps: [
        { kind: 'review_run', name: 'review' },
        { kind: 'check', name: 'check' },
        { kind: 'agent_prompt', name: 'fix' },
      ],
    },
  };
  const mkCp = (node_id: string, iteration: number, step_index: number, kind: string, name: string, status: string, output: unknown = null, extra: Record<string, unknown> = {}) =>
    ({ node_id, loop_id: 'ui', iteration, step_index, kind, name, status, attempts: 1, input: null, output, logs: [], updated_at: at(iteration), ...extra }) as never;

  const checkpoints = [
    mkCp('ui#1.0#review.0', 1, 0, 'review_run', 'review — app', 'success', { review_id: 'rev-1' }),
    mkCp('ui#1.0', 1, 0, 'review_run', 'review', 'success', { review_id: 'rev-1', score: 35, threshold: 95, open: 68 }),
    mkCp('ui#1.1', 1, 1, 'check', 'check', 'success', { criteria: [{ id: 'review-passes', met: false, progress: { value: 35, target: 95 } }], all_met: false, met: 0, total: 1 }),
    mkCp('ui#1.2', 1, 2, 'agent_prompt', 'fix', 'success', {}, { thread_id: 'th-fix-1' }),
    mkCp('ui#2.0#review.0', 2, 0, 'review_run', 'review — app', 'success', { review_id: 'rev-2' }),
    mkCp('ui#2.0', 2, 0, 'review_run', 'review', 'success', { review_id: 'rev-2', score: 58, threshold: 95, open: 14 }),
    mkCp('ui#2.1', 2, 1, 'check', 'check', 'success', { criteria: [{ id: 'review-passes', met: false, progress: { value: 58, target: 95 } }], all_met: false, met: 0, total: 1 }),
    mkCp('ui#2.2', 2, 2, 'agent_prompt', 'fix', 'success', {}, { thread_id: 'th-fix-2' }),
    mkCp('ui#3.0#review.0', 3, 0, 'review_run', 'review — app', 'running', { review_id: 'rev-3' }),
  ];

  // Round 3 has 12 reviewers: 9 done, 2 running, 1 queued, plus its own pending Summarizer.
  const round3Reviewers = [
    ...Array.from({ length: 9 }, (_, i) => ({ id: `rev-3.${i}`, label: `Lens ${i + 1}`, provider: 'codex', status: 'success' as const, note: '2 findings', thread_id: `t3-${i}` })),
    { id: 'rev-3.9', label: 'Security · Claude', provider: 'claude', status: 'running' as const, started_at: at(8), thread_id: 't3-9' },
    { id: 'rev-3.10', label: 'Correctness · Codex', provider: 'codex', status: 'running' as const, started_at: at(9), thread_id: 't3-10' },
    { id: 'rev-3.11', label: 'Performance · Gemini', provider: 'gemini', status: 'queued' as const },
    { id: 'rev-3.12', label: 'Summarizer', provider: 'claude', status: 'pending' as const },
  ];

  const loopNode = node('ui', 'running', 0, null, {
    logs: ['🔁 iteration 1/5', '🔁 iteration 2/5', '🔁 iteration 3/5'],
    review_ids: ['rev-1', 'rev-2', 'rev-3'],
    children: [
      { id: 'rev-1.0', label: 'Correctness', provider: 'codex', status: 'success', note: '68 findings' },
      { id: 'rev-1.1', label: 'Summarizer', provider: 'claude', status: 'success', note: 'Done' },
      { id: 'th-fix-1', label: 'fix · iteration 1', provider: 'claude', status: 'success', thread_id: 'th-fix-1' },
      { id: 'rev-2.0', label: 'Correctness', provider: 'codex', status: 'success', note: '14 findings' },
      { id: 'rev-2.1', label: 'Summarizer', provider: 'claude', status: 'success', note: 'Done' },
      { id: 'th-fix-2', label: 'fix · iteration 2', provider: 'claude', status: 'success', thread_id: 'th-fix-2' },
      ...round3Reviewers,
    ],
  });

  const goal = {
    iteration: 3,
    max_iterations: 5,
    criteria_met: 0,
    criteria_total: 1,
    active_ms: 0,
    criteria: [
      {
        step_id: 'ui.1',
        step_name: 'UI',
        id: 'review-passes',
        progress: { value: 58, target: 95 },
        history: [
          { iteration: 1, met: false, value: 35 },
          { iteration: 2, met: false, value: 58 },
        ],
      },
    ],
  };

  const view = groupLoopRounds({ id: 'ui', kind: 'loop', node: loopNode }, { graphNode: loopGraphNode, checkpoints, goal })!;
  assert.equal(view.maxIterations, 5);
  assert.equal(view.currentIteration, 3);
  assert.equal(view.rounds.length, 3);
  assert.equal(view.ungrouped.length, 0);

  // Earlier rounds are not current and have one-line summaries with score, comments, and fix outcome.
  assert.equal(view.rounds[0].current, false);
  assert.equal(view.rounds[0].scoreText, '35 / 95');
  assert.equal(view.rounds[0].summary, '35 / 95 · 68 comments · fixed');
  assert.deepEqual(view.rounds[0].steps.map((s) => s.name), ['review', 'check', 'fix']);
  // Each round's Summarizer stays nested inside THAT round's review step (never loose at the bottom).
  assert.deepEqual(view.rounds[0].steps[0].children.trailing.map((e) => e.child.id), ['rev-1.1']);
  assert.deepEqual(view.rounds[1].steps[0].children.trailing.map((e) => e.child.id), ['rev-2.1']);
  assert.deepEqual(view.rounds[2].steps[0].children.trailing.map((e) => e.child.id), ['rev-3.12']);

  // Current round is Round 3, open and running, with 9 of 12 reviewers done and waiting reviewers named.
  assert.equal(view.rounds[2].current, true);
  assert.equal(view.rounds[2].steps[0].status, 'running');
  assert.equal(view.rounds[2].steps[0].summary, '9 of 12 reviewers done');
  assert.equal(loopPositionLine({ kind: 'loop', node: loopNode }, view), 'Round 3 of 5 · reviewing (9 of 12 reviewers done)');

  const revWait = waitingReviewers(view.rounds[2].steps[0].allChildren);
  assert.deepEqual(revWait.running.map((r) => [r.childId, r.label, r.provider]), [
    ['rev-3.9', 'Security · Claude', 'claude'],
    ['rev-3.10', 'Correctness · Codex', 'codex'],
  ]);
  assert.equal(revWait.queued, 1);
});

test('groupLoopRounds keeps a retried fix attempt inside its round and supports two independent loops in one run', () => {
  const graph: WorkflowGraph = {
    nodes: [
      { id: 'start', kind: 'manual_trigger', name: 'Start', x: 0, y: 0, params: {} },
      { id: 'ui', kind: 'loop', name: 'UI', x: 0, y: 100, params: { max_iterations: 5, steps: [{ kind: 'review_run', name: 'review' }, { kind: 'check', name: 'check' }, { kind: 'agent_prompt', name: 'fix' }] } },
      { id: 'server', kind: 'loop', name: 'server', x: 200, y: 100, params: { max_iterations: 5, steps: [{ kind: 'review_run', name: 'review' }, { kind: 'check', name: 'check' }, { kind: 'agent_prompt', name: 'fix' }] } },
    ],
    edges: [{ id: 'e1', source: 'start', target: 'ui' }, { id: 'e2', source: 'start', target: 'server' }],
  };
  const uiChildren = annotateRetriedChildren([
    { id: 'u-fix-1a', label: 'fix · iteration 1', provider: 'claude', status: 'error', thread_id: 'u-fix-1a', depth: 0, parent_child_id: null },
    { id: 'u-fix-1b', label: 'fix · iteration 1', provider: 'claude', status: 'success', thread_id: 'u-fix-1b', depth: 0, parent_child_id: null },
    { id: 'u-fix-2', label: 'fix · iteration 2', provider: 'claude', status: 'running', thread_id: 'u-fix-2', depth: 0, parent_child_id: null },
  ], { loopId: 'ui', nodeLogs: ['  [1/fix] attempt 1: timed out'] });

  const uiNode = node('ui', 'running', 0, null, { logs: ['🔁 iteration 2/5'], children: uiChildren });
  const serverNode = node('server', 'running', 0, null, {
    logs: ['🔁 iteration 1/5'],
    children: [{ id: 's-rev.0', label: 'Correctness', provider: 'codex', status: 'success' }],
  });
  const serverCps = [
    { node_id: 'server#1.0', loop_id: 'server', iteration: 1, step_index: 0, kind: 'review_run', name: 'review', status: 'success', attempts: 1, input: null, output: { review_id: 's-rev', score: 81, threshold: 95 }, logs: [], updated_at: at(1) } as never,
    { node_id: 'server#1.1', loop_id: 'server', iteration: 1, step_index: 1, kind: 'check', name: 'check', status: 'running', attempts: 1, input: null, output: null, logs: [], updated_at: at(2) } as never,
  ];

  const uiView = groupLoopRounds({ id: 'ui', kind: 'loop', node: uiNode }, { graphNode: graph.nodes[1], checkpoints: serverCps })!;
  assert.equal(uiView.rounds.length, 2);
  // Retried attempt 1 stays inside Round 1's fix step (in its done fold), never leaking to Round 2.
  const r1Fix = uiView.rounds[0].steps.find((s) => s.name === 'fix')!;
  assert.deepEqual(r1Fix.allChildren.map((c) => [c.id, c.retried_attempt ?? null]), [['u-fix-1a', 1], ['u-fix-1b', null]]);
  assert.equal(r1Fix.status, 'success');
  assert.equal(loopPositionLine({ kind: 'loop', node: uiNode }, uiView), 'Round 2 of 5 · fixing');

  const serverView = groupLoopRounds({ id: 'server', kind: 'loop', node: serverNode }, { graphNode: graph.nodes[2], checkpoints: serverCps })!;
  assert.equal(serverView.rounds.length, 1);
  assert.equal(serverView.rounds[0].scoreText, '81 / 95');
  assert.equal(loopPositionLine({ kind: 'loop', node: serverNode }, serverView), 'Round 1 of 5 · checking');

  // Non-loop steps return null (runs with no loops are untouched).
  assert.equal(groupLoopRounds({ id: 'n7', kind: 'review_run', node: node('n7', 'running', 0, null) }), null);
});

test('isWaitingOnSubagents and waitingThreads name the live threads a step is waiting on', () => {
  const stepNode = node('loop', 'running', 0, null, {
    phase: 'finishing',
    activity: {
      phase: 'waiting on 2 sub-agent(s)',
      updated_at: at(10),
      pending_tasks: 2,
      subagents: [
        { id: 'th-a', description: 'Check API contract', status: 'running', started_at: at(5) },
        { id: 'sa-extra', description: 'Run smoke tests', status: 'running', started_at: at(8) },
      ],
    },
    children: [
      { id: 'c-a', label: 'Check API contract', provider: 'codex', status: 'running', thread_id: 'th-a', started_at: at(5) },
      { id: 'c-done', label: 'Done helper', provider: 'claude', status: 'success', thread_id: 'th-done' },
    ],
  });
  assert.equal(isWaitingOnSubagents(stepNode), true);
  const refs = waitingThreads(stepNode);
  assert.deepEqual(refs, [
    { childId: 'c-a', label: 'Check API contract', provider: 'codex', startedAt: at(5) },
    { childId: null, label: 'Run smoke tests', provider: '', startedAt: at(8) },
  ]);
  assert.equal(isWaitingOnSubagents(node('ok', 'success', 0, 1000)), false);
});

test('reuseMergedNode reflects in-place live node mutations while reusing unchanged merged children (BUG-15)', () => {
  const baseNode = node('n3', 'running', 1, null, {
    phase: 'working',
    detail_version: 'v1',
    thread: { thread_id: 'th-root', runner: 'threads', provider: 'claude', turns: 1, usage: {} as never },
    children: [{ id: 'c1', label: 'Helper', provider: 'codex', status: 'success', thread_id: 'th-kid' }],
  });
  const run: WorkflowRun = {
    id: 'run-1',
    workflow_id: 'wf-1',
    status: 'running',
    summary: true,
    rev: 1,
    waiting_approval: false,
    input: null,
    output: null,
    error: null,
    started_at: at(0),
    ended_at: null,
    nodes: [baseNode],
    checkpoints: [],
  } as unknown as WorkflowRun;

  const merged1 = [{ id: 'c1', label: 'Helper', provider: 'codex', status: 'success' as const, thread_id: 'th-kid', depth: 0, parent_child_id: null }];
  const entry1 = reuseMergedNode(run.nodes[0], merged1, undefined);
  assert.notEqual(entry1.node, run.nodes[0], 'merged children created a wrapper copy');
  assert.equal(entry1.node.phase, 'working');

  // Unrelated runtime-session tick with structurally identical children preserves exact node reference.
  const mergedTick = merged1.map((c) => ({ ...c }));
  const entryTick = reuseMergedNode(run.nodes[0], mergedTick, entry1);
  assert.equal(entryTick, entry1);
  assert.equal(entryTick.node, entry1.node);

  // Live event mutates `run.nodes[0]` in place (same object identity) to `needs_you` with unchanged children.
  const applied = applyLiveNode(run, {
    rev: 2,
    status: 'running',
    waitingApproval: true,
    overview: { state: 'needs_you', headline: 'Needs approval' } as never,
    node: {
      ...baseNode,
      phase: 'needs_you',
      detail_version: 'v2',
      attention: { kind: 'approval', text: 'Approve command', since: at(2) },
    },
  });
  assert.equal(applied, true);
  assert.equal(run.nodes[0], baseNode, 'applyLiveNode mutated the node in place');
  assert.equal(sameNodeFields(run.nodes[0], entry1.node), false);

  const entry2 = reuseMergedNode(run.nodes[0], merged1.map((c) => ({ ...c })), entry1);
  assert.equal(entry2.node.phase, 'needs_you');
  assert.equal(entry2.node.attention?.text, 'Approve command');
  assert.equal(entry2.children, entry1.children, 'unchanged merged children array is still reused');

  // Progress poll settles the step in place and drops `thread`.
  const settledSummary: WorkflowRun = {
    ...run,
    rev: 3,
    status: 'success',
    nodes: [
      {
        ...baseNode,
        status: 'success',
        phase: undefined,
        attention: undefined,
        thread: undefined,
        duration_ms: 4200,
        detail_version: 'v3',
      },
    ],
  };
  assert.equal(mergeRunProgress(run, settledSummary), true);
  assert.equal(run.nodes[0], baseNode, 'mergeRunProgress mutated the node in place');
  const entry3 = reuseMergedNode(run.nodes[0], merged1.map((c) => ({ ...c })), entry2);
  assert.equal(entry3.node.status, 'success');
  assert.equal(entry3.node.duration_ms, 4200);
  assert.equal(entry3.node.thread, undefined, 'deleted field is no longer retained on cached node');
  assert.equal(entry3.children, entry2.children);
});

test('annotateRetriedChildren keeps independent same-named failed threads active without checkpoint retry evidence (BUG-16)', () => {
  // Agent opens two independent child threads with the same fallback/title; first fails, second runs.
  const agentKids = [
    { id: 'th-a', label: 'codex', provider: 'codex', status: 'error' as const, thread_id: 'th-a', depth: 0, parent_child_id: null },
    { id: 'th-b', label: 'codex', provider: 'codex', status: 'running' as const, thread_id: 'th-b', depth: 0, parent_child_id: null },
    { id: 'th-c', label: 'Check tests', provider: 'claude', status: 'error' as const, thread_id: 'th-c', depth: 0, parent_child_id: null },
    { id: 'th-d', label: 'Check tests', provider: 'claude', status: 'running' as const, thread_id: 'th-d', depth: 0, parent_child_id: null },
  ];
  // Even if the parent step's own logs carry a bare `attempt 1:` line, independent child threads are NOT retried attempts.
  const annotated = annotateRetriedChildren(agentKids, {
    loopId: 'agent-step',
    nodeLogs: ['attempt 1: parent step retried once'],
  });
  assert.equal(annotated, agentKids, 'returns original array untouched when no checkpoint retry evidence exists');
  assert.deepEqual(childWord(annotated[0], NOW), { word: 'Failed', tone: 'danger' });
  assert.deepEqual(childWord(annotated[2], NOW), { word: 'Failed', tone: 'danger' });
  const tree = orderChildTree(annotated);
  assert.deepEqual(tree.active.map((e) => e.child.id), ['th-a', 'th-b', 'th-c', 'th-d']);
  assert.equal(tree.done.length, 0);

  // Nested agent sub-threads under a loop iteration are never treated as checkpoint retries even if their label matches.
  const loopCpBase = {
    node_id: 'loop#1.0',
    loop_id: 'loop',
    iteration: 1,
    step_index: 0,
    kind: 'agent_prompt',
    name: 'fix',
    status: 'success',
    attempts: 2,
    input: null,
    output: null,
    logs: ['attempt 1: timed out'],
    updated_at: at(1),
  };
  const loopCp = loopCpBase as never;
  const nestedKids = [
    { id: 'iter-1', label: 'fix · iteration 1', provider: 'claude', status: 'running' as const, thread_id: 'iter-1', depth: 0, parent_child_id: null },
    { id: 'sub-1', label: 'fix', provider: 'codex', status: 'error' as const, thread_id: 'sub-1', depth: 1, parent_child_id: 'iter-1' },
    { id: 'sub-2', label: 'fix', provider: 'codex', status: 'running' as const, thread_id: 'sub-2', depth: 1, parent_child_id: 'iter-1' },
  ];
  assert.equal(annotateRetriedChildren(nestedKids, { loopId: 'loop', checkpoints: [loopCp] }), nestedKids);

  // Even an iteration-shaped label requires checkpoint `attempts > attempt` or attempt logs.
  const singleAttemptCp = { ...loopCpBase, attempts: 1, logs: [] } as never;
  const unconfirmed = [
    { id: 'x1', label: 'fix · iteration 1', provider: 'claude', status: 'error' as const, thread_id: 'x1', depth: 0, parent_child_id: null },
    { id: 'x2', label: 'fix · iteration 1', provider: 'claude', status: 'running' as const, thread_id: 'x2', depth: 0, parent_child_id: null },
  ];
  assert.equal(annotateRetriedChildren(unconfirmed, { loopId: 'loop', checkpoints: [singleAttemptCp] }), unconfirmed);

  // A checkpoint with `attempts: 2` confirms the retry even before attempt logs are populated.
  const countOnlyCp = { ...loopCpBase, attempts: 2, logs: [] } as never;
  const confirmed = annotateRetriedChildren(unconfirmed, { loopId: 'loop', checkpoints: [countOnlyCp] });
  assert.equal(confirmed[0].retried_attempt, 1);
  assert.equal(confirmed[0].retried_reason, undefined);
  assert.deepEqual(childWord(confirmed[0], NOW), { word: 'Attempt 1 failed · retried', tone: '' });
  assert.deepEqual(orderChildTree(confirmed).done.map((e) => e.child.id), ['x1']);
});
