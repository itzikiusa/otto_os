// The "Running" workflow list moved out of the workspace store into
// lib/stores/workflowRuns.svelte.ts (architecture-2-03). Its rules: patch a
// known run in place, drop it on a terminal status, refetch for an unknown run,
// ignore other workspaces, and never let a late response overwrite a newer one.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

function fixture() {
  const calls: { wsId: string; result: ReturnType<typeof deferred<any[]>> }[] = [];
  const { workflowRuns } = loadSource(new URL('../src/lib/stores/workflowRuns.svelte.ts', import.meta.url), {
    '../api/workflows': { listActiveWorkflowRuns: (wsId: string) => { const result = deferred<any[]>(); calls.push({ wsId, result }); return result.promise; } },
  });
  return { workflowRuns, calls };
}

const run = (id: string, status = 'running') => ({ run_id: id, workflow_id: 'wf', status, nodes_done: 0, nodes_total: 3, waiting_approval: false, started_at: 't' });

test('a late response for the previous workspace never replaces the current list', async () => {
  const { workflowRuns, calls } = fixture();
  const a = workflowRuns.refresh('A');
  const b = workflowRuns.refresh('B');
  calls[1].result.resolve([run('b1')]);
  await b;
  calls[0].result.resolve([run('a1')]);
  await a;
  assert.deepEqual(workflowRuns.list.map((r: any) => r.run_id), ['b1']);
  workflowRuns.clear();
  assert.equal(workflowRuns.list.length, 0);
});

test('events patch known runs in place, drop terminal ones and refetch unknown ones', async () => {
  const { workflowRuns, calls } = fixture();
  const first = workflowRuns.refresh('A');
  calls[0].result.resolve([run('r1'), run('r2')]);
  await first;
  workflowRuns.apply({ workspace_id: 'A', run_id: 'r1', status: 'running', nodes_done: 2, nodes_total: 3, waiting_approval: true }, 'A');
  assert.equal(workflowRuns.list[0].nodes_done, 2);
  assert.equal(workflowRuns.list[0].waiting_approval, true);
  workflowRuns.apply({ workspace_id: 'A', run_id: 'r2', status: 'success' }, 'A');
  assert.deepEqual(workflowRuns.list.map((r: any) => r.run_id), ['r1']);
  workflowRuns.apply({ workspace_id: 'B', run_id: 'x', status: 'running' }, 'A');
  assert.equal(calls.length, 1, 'another workspace\'s event is ignored');
  workflowRuns.apply({ workspace_id: 'A', run_id: 'new', status: 'running' }, 'A');
  assert.equal(calls.length, 2, 'an unknown run triggers one refetch');
  assert.equal(calls[1].wsId, 'A');
});
