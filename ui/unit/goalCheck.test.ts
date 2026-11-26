// A goal run's criteria rows (goal.md §3.5, GL8.1) — workflows/goal/goalCheck.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { criterionRows, graphCriteria, latestCheck, newestCheckpoint, readsCheckpoints } from '../src/modules/workflows/goal/goalCheck.ts';
import type { CheckOutput, Criterion, NodeRunState, WorkflowCheckpoint } from '../src/lib/api/types.ts';

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
