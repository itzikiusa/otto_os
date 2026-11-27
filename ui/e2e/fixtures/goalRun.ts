// Invented goal-run fixtures for the goal e2e specs (goal.md §3.5, test-plan.md
// GL8.1 / GL21): a goal workflow (Start → loop { Work → Check criteria }), a run
// of it in a chosen state, and the check's checkpoint output. No real data.
import type { Page, Route } from '@playwright/test';

export const GOAL_WF = 'wf-goal';
export const GOAL_RUN = 'run-goal';

export const CRITERIA = [
  { id: 'tests', text: 'The import tests pass', verify: 'command', how: '', command: 'go test ./import/...' },
  { id: 'docs', text: 'The import guide describes the new rule', verify: 'agent', how: 'read docs/import.md' },
  { id: 'ui', text: 'The error banner reads well', verify: 'human', how: 'open the import page' },
];

export function goalWorkflow(workspaceId: string) {
  return {
    id: GOAL_WF, workspace_id: workspaceId, name: 'Goal · Reject malformed rows', description: 'Reject malformed rows', instructions: '',
    origin: 'goal', version: 1, created_by: 'u', created_at: '2026-11-20T09:00:00Z', updated_at: '2026-11-20T09:00:00Z',
    graph: {
      nodes: [
        { id: 'start', kind: 'manual_trigger', name: 'Start', x: 0, y: 0, params: { lane: 'long' } },
        { id: 'loop', kind: 'loop', name: 'Work toward the goal', x: 0, y: 120, params: { max_iterations: 6, max_active_s: 7200, hold_when_unmet: true,
          steps: [{ id: 'work', kind: 'agent_prompt', name: 'Work', params: { same_thread: true } }, { id: 'check', kind: 'check', name: 'Check criteria', params: { criteria: CRITERIA } }] } },
      ],
      edges: [{ id: 'e1', source: 'start', target: 'loop' }],
    },
  };
}

export const CHECK_OUTPUT = {
  criteria: [
    { id: 'tests', text: CRITERIA[0].text, verify: 'command', met: false, evidence: 'verify command failed (exit 1)\n--- verify output ---\n--- FAIL: TestRejectsBlankRow\nFAIL import 0.21s', verified_by: 'command' },
    { id: 'docs', text: CRITERIA[1].text, verify: 'agent', met: true, evidence: 'docs/import.md §3 describes the rule\nLines 40-58 list the three rejected shapes.', verified_by: 'agent' },
  ],
  all_met: false, met: 1, total: 3, progress_pct: 33, verdict: 'continue', feedback: 'Fix the blank-row case.', feedback_md: '', repeat_signature: 'tests', repeated_failures: 1,
};

type State = 'running' | 'needs_you' | 'blocked' | 'exhausted' | 'succeeded';

export function goalRun(workspaceId: string, state: State) {
  const held = state === 'blocked' || state === 'exhausted';
  const status = state === 'succeeded' ? 'success' : held ? 'error' : 'running';
  const hold = held ? { kind: state, reason: state === 'exhausted' ? 'Out of iterations: 6 of 6 used' : 'Two checks in a row failed the same way — a new approach is needed', since: '2026-11-20T10:10:00Z', node_ids: ['loop'], iteration: 6 } : undefined;
  const attention = state === 'needs_you'
    ? [{ kind: 'verify', text: 'the error banner reads well', since: '2026-11-20T10:05:00Z', node_id: 'loop', name: 'Work toward the goal', criterion_id: 'ui' }]
    : [];
  return {
    id: GOAL_RUN, workflow_id: GOAL_WF, workspace_id: workspaceId, status, input: { goal: { title: 'Reject malformed rows' }, repos: [{ repo: 'r1', type: 'worktree', name: '/src/app', source: 'abc123' }] },
    started_at: '2026-11-20T09:00:00Z', finished_at: status === 'running' ? null : '2026-11-20T10:10:00Z', rev: 7, workflow_version: 1,
    resume_attempts: 0, checkpoint_rev: 1, checkpoint_generation: 0, waiting_approval: false,
    hold, steering_queued: state === 'running' ? ['Also cover the CRLF case'] : [],
    nodes: [
      { node_id: 'start', status: 'success', logs: [], step_no: 1 },
      { node_id: 'loop', status: status === 'success' ? 'success' : status === 'running' ? 'running' : 'pending', logs: [], step_no: 2, phase: status === 'running' ? (state === 'needs_you' ? 'needs_you' : 'working') : undefined,
        attention: attention[0] ? { kind: 'verify', text: attention[0].text, since: attention[0].since, criterion_id: 'ui' } : undefined },
    ],
    checkpoints: [
      { node_id: 'loop#3.1', loop_id: 'loop', iteration: 3, step_index: 1, kind: 'check', name: 'Check criteria', status: 'success', attempts: 1, input: null, output: CHECK_OUTPUT, logs: [], updated_at: '2026-11-20T10:00:00Z' },
    ],
    overview: {
      rev: 7, state, headline: state === 'succeeded' ? 'Goal reached — 3 of 3 criteria met in 4 iterations' : 'Iteration 4 of 6 — Work: editing import/rows.go',
      steps_done: 1, steps_total: 2, attention, deliverables: [], usage: {}, time: { queued_ms: 0, waiting_ms: 0, with_you_ms: 0, working_ms: 0 }, elapsed_ms: 4_200_000,
      goal: { iteration: state === 'exhausted' ? 6 : 4, max_iterations: 6, criteria_met: 1, criteria_total: 3, active_ms: 4_320_000, max_active_ms: 7_200_000, awaiting: state === 'needs_you' ? ['ui'] : [] },
      ...(hold ? { hold } : {}),
    },
  };
}

/** `GET /workflow-runs/{id}/progress` as the server builds it
 *  (crates/otto-state/src/workflow_progress.rs): the envelope, the run's
 *  scalar columns, node summaries, `summary: true`, `input: null`, no
 *  checkpoint bodies — only their counts. */
export function progressOf(run: ReturnType<typeof goalRun>) {
  const { input: _input, checkpoints, steering_queued: _steer, ...cols } = run;
  return {
    changed: true, rev: run.rev,
    run: {
      ...cols, summary: true, input: null, checkpoints: [],
      checkpoint_count: checkpoints.length, checkpoint_done: checkpoints.filter((c) => c.status === 'success').length,
      nodes: run.nodes.map((n) => ({ ...n, logs: [], log_count: n.logs.length, has_output: false, detail_version: `${n.node_id}-${n.status}` })),
    },
  };
}

/** Serve the goal workflow and one run of it; record every write. */
export async function serveGoalRun(page: Page, workspaceId: string, state: State): Promise<{ writes: { path: string; body: unknown }[] }> {
  const writes: { path: string; body: unknown }[] = [];
  const wf = goalWorkflow(workspaceId);
  const run = goalRun(workspaceId, state);
  await page.route(/\/api\/v1\/(workflow-runs|workflows)\//, async (route: Route) => {
    const req = route.request();
    const path = new URL(req.url()).pathname.replace('/api/v1', '');
    if (req.method() !== 'GET') {
      writes.push({ path, body: req.postDataJSON() });
      return route.fulfill({ json: run });
    }
    if (path === `/workflow-runs/${GOAL_RUN}`) return route.fulfill({ json: run });
    if (path === `/workflow-runs/${GOAL_RUN}/progress`) return route.fulfill({ json: progressOf(run) });
    if (path === `/workflow-runs/${GOAL_RUN}/checkpoints`) return route.fulfill({ json: { checkpoint_rev: run.checkpoint_rev, generation: run.checkpoint_generation, items: run.checkpoints, next_cursor: null } });
    if (path.startsWith(`/workflows/${GOAL_WF}`)) return route.fulfill({ json: wf });
    return route.fallback();
  });
  return { writes };
}

export const MULTI_LOOP_WF = 'wf-goal-multi';
export const MULTI_LOOP_RUN = 'run-goal-multi';

/** Serve a run with two review-and-fix loops whose check steps share the same
 *  criterion id (`review-passes`), each with score progress and round history. */
export async function serveMultiLoopGoalRun(page: Page, workspaceId: string): Promise<void> {
  const reviewCriterion = { id: 'review-passes', text: 'A fresh independent review passes', verify: 'expr', how: '', expr: 'steps.review.passed == true' };
  const wf = {
    id: MULTI_LOOP_WF, workspace_id: workspaceId, name: 'Review & fix (UI + server)', description: 'Score 95+ in both loops', instructions: '',
    origin: 'goal', version: 1, created_by: 'u', created_at: '2026-11-20T09:00:00Z', updated_at: '2026-11-20T09:00:00Z',
    graph: {
      nodes: [
        { id: 'start', kind: 'manual_trigger', name: 'Start', x: 0, y: 0, params: { lane: 'long' } },
        { id: 'ui', kind: 'loop', name: 'UI', x: 0, y: 120, params: { max_iterations: 5, steps: [{ kind: 'review_run', name: 'review' }, { kind: 'check', name: 'check', params: { criteria: [reviewCriterion] } }] } },
        { id: 'server', kind: 'loop', name: 'server', x: 240, y: 120, params: { max_iterations: 5, steps: [{ kind: 'review_run', name: 'review' }, { kind: 'check', name: 'check', params: { criteria: [reviewCriterion] } }] } },
      ],
      edges: [{ id: 'e1', source: 'start', target: 'ui' }, { id: 'e2', source: 'start', target: 'server' }],
    },
  };
  const mkCheck = (met: boolean, score: number, label?: string) => ({
    criteria: [{
      id: 'review-passes', text: reviewCriterion.text, verify: 'expr', met,
      evidence: `steps.review.passed == true is ${met} — ${score} / 95${label ? `, ${label}` : ''}`, verified_by: 'rule',
      progress: { value: score, target: 95, ...(label ? { label } : {}), values: { 'steps.review.passed': met, score, threshold: 95 } },
    }],
    all_met: met, met: met ? 1 : 0, total: 1, progress_pct: met ? 100 : 0, verdict: met ? 'achieved' : 'continue',
    feedback: '', feedback_md: '', repeat_signature: '', repeated_failures: 0,
  });
  const checkpoints = [
    { node_id: 'ui#1.1', loop_id: 'ui', iteration: 1, step_index: 1, kind: 'check', name: 'check', status: 'success', attempts: 1, input: null, output: mkCheck(false, 35, '4 must-fix findings left'), logs: [], updated_at: '2026-11-20T09:10:00Z' },
    { node_id: 'ui#2.1', loop_id: 'ui', iteration: 2, step_index: 1, kind: 'check', name: 'check', status: 'success', attempts: 1, input: null, output: mkCheck(false, 58, '3 must-fix findings left'), logs: [], updated_at: '2026-11-20T09:20:00Z' },
    { node_id: 'ui#3.1', loop_id: 'ui', iteration: 3, step_index: 1, kind: 'check', name: 'check', status: 'success', attempts: 1, input: null, output: mkCheck(false, 72, '3 must-fix findings left'), logs: [], updated_at: '2026-11-20T09:30:00Z' },
    { node_id: 'server#1.1', loop_id: 'server', iteration: 1, step_index: 1, kind: 'check', name: 'check', status: 'success', attempts: 1, input: null, output: mkCheck(false, 60, '2 must-fix findings left'), logs: [], updated_at: '2026-11-20T09:10:00Z' },
    { node_id: 'server#2.1', loop_id: 'server', iteration: 2, step_index: 1, kind: 'check', name: 'check', status: 'success', attempts: 1, input: null, output: mkCheck(false, 80, '1 must-fix finding left'), logs: [], updated_at: '2026-11-20T09:20:00Z' },
    { node_id: 'server#3.1', loop_id: 'server', iteration: 3, step_index: 1, kind: 'check', name: 'check', status: 'success', attempts: 1, input: null, output: mkCheck(false, 81, '1 must-fix finding left'), logs: [], updated_at: '2026-11-20T09:30:00Z' },
  ];
  const run = {
    id: MULTI_LOOP_RUN, workflow_id: MULTI_LOOP_WF, workspace_id: workspaceId, status: 'running', input: { goal: { title: 'Review & fix' } },
    started_at: '2026-11-20T09:00:00Z', finished_at: null, rev: 9, workflow_version: 1,
    resume_attempts: 0, checkpoint_rev: 6, checkpoint_generation: 0, waiting_approval: false, steering_queued: [],
    nodes: [
      { node_id: 'start', status: 'success', logs: [], step_no: 1 },
      { node_id: 'ui', status: 'running', logs: ['🔁 iteration 3/5'], step_no: 2, phase: 'working' },
      { node_id: 'server', status: 'running', logs: ['🔁 iteration 3/5'], step_no: 3, phase: 'working' },
    ],
    checkpoints,
    overview: {
      rev: 9, state: 'running', headline: 'Round 3 of 5 — UI 72 / 95, server 81 / 95',
      steps_done: 1, steps_total: 3, attention: [], deliverables: [], usage: {}, time: { queued_ms: 0, waiting_ms: 0, with_you_ms: 0, working_ms: 0 }, elapsed_ms: 1_800_000,
      goal: {
        iteration: 3, max_iterations: 5, criteria_met: 0, criteria_total: 2, active_ms: 900_000,
        criteria: [
          {
            step_id: 'ui.1', step_name: 'UI', id: 'review-passes', text: reviewCriterion.text, met: false,
            progress: { value: 72, target: 95, label: '3 must-fix findings left', values: { 'steps.review.passed': false, score: 72, threshold: 95 } },
            history: [{ iteration: 1, met: false, value: 35 }, { iteration: 2, met: false, value: 58 }, { iteration: 3, met: false, value: 72 }],
          },
          {
            step_id: 'server.1', step_name: 'server', id: 'review-passes', text: reviewCriterion.text, met: false,
            progress: { value: 81, target: 95, label: '1 must-fix finding left', values: { 'steps.review.passed': false, score: 81, threshold: 95 } },
            history: [{ iteration: 1, met: false, value: 60 }, { iteration: 2, met: false, value: 80 }, { iteration: 3, met: false, value: 81 }],
          },
        ],
      },
    },
  };
  await page.route(/\/api\/v1\/(workflow-runs|workflows)\//, async (route: Route) => {
    const path = new URL(route.request().url()).pathname.replace('/api/v1', '');
    if (path === `/workflow-runs/${MULTI_LOOP_RUN}`) return route.fulfill({ json: run });
    if (path === `/workflow-runs/${MULTI_LOOP_RUN}/progress`) return route.fulfill({ json: progressOf(run as unknown as ReturnType<typeof goalRun>) });
    if (path === `/workflow-runs/${MULTI_LOOP_RUN}/checkpoints`) return route.fulfill({ json: { checkpoint_rev: run.checkpoint_rev, generation: 0, items: run.checkpoints, next_cursor: null } });
    if (path.startsWith(`/workflows/${MULTI_LOOP_WF}`)) return route.fulfill({ json: wf });
    return route.fallback();
  });
}
