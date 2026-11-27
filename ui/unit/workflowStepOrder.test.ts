// The run page's step list (ux.md §4.2, §4.4): execution order, parallel
// groups, a fan-out's children, the phone's done fold and the pane's default
// selection — runs/stepOrder.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { annotateRetriedChildren, childCrumb, childNoun, childTrail, defaultSelection, doneFold, entrySteps, loopChildren, mergeStepChildren, orderChildren, orderChildTree, orderSteps, resolveThreadSelection, RESULT, retriedAttemptReason, runtimeChildStatus, sameStepChildren, settledNodes, stepNumber, timeBar, topoOrder } from '../src/modules/workflows/runs/stepOrder.ts';
import { paneTabs } from '../src/modules/workflows/runs/stepTabs.ts';
import { at, fanoutNodes, node, PR_GRAPH, reviewers, T0 } from './workflowRunFixtures.ts';
import { childWord, stepWord } from '../src/modules/workflows/runs/runWords.ts';
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
