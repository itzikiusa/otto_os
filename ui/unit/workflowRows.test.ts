import { test } from 'node:test';
import assert from 'node:assert/strict';
import { splitGoals, workflowRow } from '../src/modules/workflows/list/workflowRows.ts';

const now = Date.parse('2026-11-26T12:00:00Z');
const graph = { nodes: [{ id: 't', kind: 'manual_trigger', name: 'Start', x: 0, y: 0, params: null }, { id: 'a', kind: 'agent_prompt', name: 'A', x: 0, y: 0, params: null }], edges: [] };
const base = { graph, description: '' };

test('a workflow that waits on a person says so before anything else', () => {
  const r = workflowRow({ ...base, running_count: 1, last_run: { id: 'r', status: 'running', overview_state: 'needs_you', attention: true } }, now);
  assert.deepEqual([r.word, r.tone], ['Needs you', 'warning']);
  const held = workflowRow({ ...base, last_run: { id: 'r', status: 'error', overview_state: 'exhausted' } }, now);
  assert.deepEqual([held.word, held.tone], ['Out of budget', 'warning']);
});

test('running, failed and finished rows', () => {
  assert.equal(workflowRow({ ...base, running_count: 1, last_run: { id: 'r', status: 'running', overview_state: 'running' } }, now).word, 'Working');
  assert.equal(workflowRow({ ...base, running_count: 3, last_run: { id: 'r', status: 'running' } }, now).word, '3 running');
  const failed = workflowRow({ ...base, last_run: { id: 'r', status: 'error', ended_at: '2026-11-26T11:00:00Z', duration_ms: 125_000 } }, now);
  assert.deepEqual([failed.word, failed.tone, failed.time], ['Failed', 'danger', '1h ago']);
  assert.equal(failed.line3, '1 step · last run took 2m 05s');
  // A finished run shows when it ended, not "Done".
  const done = workflowRow({ ...base, description: 'Reviews a PR\nmore', last_run: { id: 'r', status: 'success', overview_state: 'succeeded', ended_at: '2026-11-26T11:58:00Z' } }, now);
  assert.deepEqual([done.word, done.time, done.line3], [null, '2m ago', 'Reviews a PR']);
  assert.equal(workflowRow(base, now).line3, '1 step · No runs yet');
});

test('goals fold apart, both sorted by name', () => {
  const { workflows, goals } = splitGoals([{ name: 'b' }, { name: 'g', origin: 'goal' }, { name: 'a' }]);
  assert.deepEqual(workflows.map((w) => w.name), ['a', 'b']);
  assert.deepEqual(goals.map((w) => w.name), ['g']);
});
