import test from 'node:test';
import assert from 'node:assert/strict';
import { groupLoopRounds, waitingReviewers } from '../src/modules/workflows/runs/loopRounds.ts';
import { checkpointStamp, mergeCheckpointDetails, needsCheckpointDetail, needsCheckpointRefresh } from '../src/modules/workflows/runCheckpoints.ts';
import type { StepChild } from '../src/modules/workflows/runs/stepOrder.ts';
import type { NodeRunState, WorkflowCheckpoint, WorkflowNode } from '../src/lib/api/types';

function cp(over: Partial<WorkflowCheckpoint> & Pick<WorkflowCheckpoint, 'node_id' | 'loop_id' | 'iteration' | 'step_index' | 'kind' | 'name'>): WorkflowCheckpoint {
  return {
    status: 'success',
    attempts: 1,
    input: null,
    output: null,
    logs: [],
    updated_at: '2026-11-27T10:00:00Z',
    ...over,
  };
}

test('BUG-6: pass-through delay/condition step cannot steal a review’s threads or throw on non-array reviews', () => {
  const graphNode: Pick<WorkflowNode, 'id' | 'kind' | 'params'> = {
    id: 'loop1',
    kind: 'loop',
    params: {
      max_iterations: 3,
      steps: [
        { kind: 'review_run', name: 'review' },
        { kind: 'delay', name: 'wait' },
        { kind: 'condition', name: 'branch' },
      ],
    },
  };
  const checkpoints: WorkflowCheckpoint[] = [
    cp({ node_id: 'loop1#1.0#review.0', loop_id: 'loop1', iteration: 1, step_index: 0, kind: 'review_run', name: 'review', output: { review_id: 'rev-1' } }),
    cp({ node_id: 'loop1#1.0', loop_id: 'loop1', iteration: 1, step_index: 0, kind: 'review_run', name: 'review', output: { review_id: 'rev-1', score: 80, threshold: 95 } }),
    // Delay and condition pass through the review output (and an arbitrary non-array `reviews` value).
    cp({ node_id: 'loop1#1.1', loop_id: 'loop1', iteration: 1, step_index: 1, kind: 'delay', name: 'wait', output: { review_id: 'rev-1', reviews: 42 } }),
    cp({ node_id: 'loop1#1.2', loop_id: 'loop1', iteration: 1, step_index: 2, kind: 'condition', name: 'branch', output: { review_id: 'rev-1', reviews: { bad: true } } }),
  ];
  const node: NodeRunState = {
    node_id: 'loop1',
    status: 'running',
    logs: ['🔁 iteration 1/3'],
    children: [
      { id: 'rev-1.0', label: 'Correctness', provider: 'codex', status: 'success' },
      { id: 'rev-1.1', label: 'Summarizer', provider: 'claude', status: 'success' },
    ],
  };

  const view = groupLoopRounds({ id: 'loop1', kind: 'loop', node }, { graphNode, checkpoints });
  assert.ok(view);
  assert.equal(view.rounds.length, 1);
  const reviewStep = view.rounds[0].steps.find((s) => s.stepIndex === 0)!;
  const delayStep = view.rounds[0].steps.find((s) => s.stepIndex === 1)!;
  const condStep = view.rounds[0].steps.find((s) => s.stepIndex === 2)!;
  assert.deepEqual(reviewStep.allChildren.map((c) => c.id), ['rev-1.0', 'rev-1.1']);
  assert.equal(delayStep.allChildren.length, 0);
  assert.equal(condStep.allChildren.length, 0);
});

test('BUG-7: shared worker thread across 10 rounds stays in round 10 even under lexical checkpoint order', () => {
  const graphNode: Pick<WorkflowNode, 'id' | 'kind' | 'params'> = {
    id: 'work_loop',
    kind: 'loop',
    params: {
      max_iterations: 12,
      steps: [
        { kind: 'agent_prompt', name: 'Work' },
        { kind: 'check', name: 'check' },
      ],
    },
  };
  // SQLite `ORDER BY node_id` returns `#10.0` BEFORE `#9.0` lexically.
  const checkpoints: WorkflowCheckpoint[] = [
    cp({ node_id: 'work_loop#1.0', loop_id: 'work_loop', iteration: 1, step_index: 0, kind: 'agent_prompt', name: 'Work', thread_id: 'th-shared' }),
    cp({ node_id: 'work_loop#10.0', loop_id: 'work_loop', iteration: 10, step_index: 0, kind: 'agent_prompt', name: 'Work', status: 'running', thread_id: 'th-shared' }),
    cp({ node_id: 'work_loop#9.0', loop_id: 'work_loop', iteration: 9, step_index: 0, kind: 'agent_prompt', name: 'Work', thread_id: 'th-shared' }),
  ];
  const node: NodeRunState = {
    node_id: 'work_loop',
    status: 'running',
    logs: ['🔁 iteration 10/12'],
    children: [
      { id: 'c-work-10', label: 'Work · iteration 10', provider: 'claude', status: 'running', thread_id: 'th-shared' },
      // Even a label-less child falling back to thread_id lands in round 10, not round 9.
      { id: 'c-work-fallback', label: 'Worker', provider: 'claude', status: 'running', thread_id: 'th-shared' },
    ],
  };

  const view = groupLoopRounds({ id: 'work_loop', kind: 'loop', node }, { graphNode, checkpoints });
  assert.ok(view);
  assert.equal(view.currentIteration, 10);
  const round9 = view.rounds.find((r) => r.iteration === 9)!;
  const round10 = view.rounds.find((r) => r.iteration === 10)!;
  assert.equal(round9.current, false);
  assert.equal(round9.steps[0].allChildren.length, 0);
  assert.equal(round10.current, true);
  assert.deepEqual(round10.steps[0].allChildren.map((c) => c.id), ['c-work-10', 'c-work-fallback']);
});

test('BUG-8: waitingReviewers scopes blockers to each summarizer’s own review id in multi-repo reviews', () => {
  const children: StepChild[] = [
    { id: 'rev-a.0', label: 'Repo A · Security', provider: 'claude', status: 'success' },
    { id: 'rev-a.1', label: 'Repo A · Summarizer', provider: 'claude', status: 'pending' },
    { id: 'rev-b.0', label: 'Repo B · Security', provider: 'codex', status: 'running', started_at: '2026-11-27T10:00:00Z' },
    { id: 'rev-b.1', label: 'Repo B · Performance', provider: 'claude', status: 'queued' },
    { id: 'rev-b.2', label: 'Repo B · Summarizer', provider: 'claude', status: 'pending' },
  ];

  // Repo A's summarizer has no running/queued blockers of its own (must NOT show Repo B's).
  const waitA = waitingReviewers(children, 'rev-a.1');
  assert.equal(waitA.running.length, 0);
  assert.equal(waitA.queued, 0);

  // Repo B's summarizer lists only Repo B's running/queued reviewers.
  const waitB = waitingReviewers(children, 'rev-b.2');
  assert.deepEqual(waitB.running.map((r) => r.childId), ['rev-b.0']);
  assert.equal(waitB.queued, 1);
});

test('ARCH-1: shared checkpoint refresh detects rev advance, retry generation bump, and fetches detail only for check/review outputs', () => {
  const initialRun = {
    id: 'run-1',
    summary: false,
    checkpoint_rev: 2,
    checkpoint_generation: 0,
    checkpoints: [
      cp({ node_id: 'lp#1.0', loop_id: 'lp', iteration: 1, step_index: 0, kind: 'review_run', name: 'review', output: { review_id: 'r1', score: 70, threshold: 95 } }),
      cp({ node_id: 'lp#1.1', loop_id: 'lp', iteration: 1, step_index: 1, kind: 'check', name: 'check', output: { criteria: [], all_met: false, met: 0, total: 1, progress_pct: 0, verdict: 'continue', feedback: '', feedback_md: '', repeat_signature: '', repeated_failures: 0 } }),
    ],
    overview: { goal: { iteration: 1, max_iterations: 5, criteria_met: 0, criteria_total: 1, active_ms: 0 } },
  };
  // Initial full read already has checkpoints at rev 2, gen 0 -> no extra page fetch needed.
  assert.equal(needsCheckpointRefresh(initialRun, null), false);
  const stamp0 = checkpointStamp(initialRun);
  assert.equal(needsCheckpointRefresh({ ...initialRun, summary: true }, stamp0), false);

  // Command-only check advances checkpoint_rev to 3 at the same generation -> must refresh.
  assert.equal(needsCheckpointRefresh({ ...initialRun, summary: true, checkpoint_rev: 3 }, stamp0), true);

  // In-place retry bumps checkpoint_generation to 1 and clears checkpoints -> must refresh.
  assert.equal(
    needsCheckpointRefresh({ ...initialRun, summary: true, checkpoint_rev: 0, checkpoint_generation: 1, checkpoints: [] }, stamp0),
    true,
  );
  // Once the new generation's empty page is loaded at (rev 0, gen 1), no further refresh is needed (BUG-19).
  assert.equal(
    needsCheckpointRefresh(
      { ...initialRun, summary: true, checkpoint_rev: 0, checkpoint_generation: 1, checkpoints: [] },
      { runId: 'run-1', rev: 0, generation: 1 },
    ),
    false,
  );

  // Selective detail fetch: only check (settled without CheckOutput) and review_run (null output) need detail;
  // agent_prompt/delay are skipped, and unchanged cached bodies are reused.
  const cached = new Map<string, WorkflowCheckpoint>([['lp#1.0', initialRun.checkpoints[0]]]);
  const paged: WorkflowCheckpoint[] = [
    // Unchanged review_run -> reused from cache.
    cp({ node_id: 'lp#1.0', loop_id: 'lp', iteration: 1, step_index: 0, kind: 'review_run', name: 'review', output: null }),
    // New review_run missing output -> fetched.
    cp({ node_id: 'lp#2.0', loop_id: 'lp', iteration: 2, step_index: 0, kind: 'review_run', name: 'review', output: null }),
    // New settled command-only check missing output -> fetched.
    cp({ node_id: 'lp#2.1', loop_id: 'lp', iteration: 2, step_index: 1, kind: 'check', name: 'check', status: 'success', output: null }),
    // Agent/delay checkpoints -> never fetched for detail.
    cp({ node_id: 'lp#2.2', loop_id: 'lp', iteration: 2, step_index: 2, kind: 'agent_prompt', name: 'fix', output: null }),
    cp({ node_id: 'lp#2.3', loop_id: 'lp', iteration: 2, step_index: 3, kind: 'delay', name: 'wait', output: null }),
  ];
  assert.equal(needsCheckpointDetail(paged[3]), false);
  assert.equal(needsCheckpointDetail(paged[4]), false);
  const { merged, toFetch } = mergeCheckpointDetails(paged, cached);
  assert.deepEqual(merged[0].output, { review_id: 'r1', score: 70, threshold: 95 });
  assert.deepEqual(toFetch.map((c) => c.node_id), ['lp#2.0', 'lp#2.1']);
});

test('BUG-20: two loop steps sharing the same name each keep their own thread via exact checkpoint thread_id', () => {
  const graphNode: Pick<WorkflowNode, 'id' | 'kind' | 'params'> = {
    id: 'lp',
    kind: 'loop',
    params: {
      max_iterations: 3,
      steps: [
        { kind: 'agent_prompt', name: 'Work' },
        { kind: 'check', name: 'check' },
        { kind: 'agent_prompt', name: 'Work' },
      ],
    },
  };
  const checkpoints: WorkflowCheckpoint[] = [
    cp({ node_id: 'lp#1.0', loop_id: 'lp', iteration: 1, step_index: 0, kind: 'agent_prompt', name: 'Work', thread_id: 'th-work-a' }),
    cp({ node_id: 'lp#1.1', loop_id: 'lp', iteration: 1, step_index: 1, kind: 'check', name: 'check' }),
    cp({ node_id: 'lp#1.2', loop_id: 'lp', iteration: 1, step_index: 2, kind: 'agent_prompt', name: 'Work', thread_id: 'th-work-b' }),
  ];
  const node: NodeRunState = {
    node_id: 'lp',
    status: 'running',
    logs: ['🔁 iteration 1/3'],
    children: [
      { id: 'c-a', label: 'Work · iteration 1', provider: 'claude', status: 'success', thread_id: 'th-work-a' },
      { id: 'c-b', label: 'Work · iteration 1', provider: 'codex', status: 'running', thread_id: 'th-work-b' },
    ],
  };

  const view = groupLoopRounds({ id: 'lp', kind: 'loop', node }, { graphNode, checkpoints });
  assert.ok(view);
  const step0 = view.rounds[0].steps.find((s) => s.stepIndex === 0)!;
  const step2 = view.rounds[0].steps.find((s) => s.stepIndex === 2)!;
  assert.deepEqual(step0.allChildren.map((c) => c.id), ['c-a']);
  assert.deepEqual(step2.allChildren.map((c) => c.id), ['c-b']);
  assert.equal(step0.childId, 'c-a');
  assert.equal(step2.childId, 'c-b');
});
