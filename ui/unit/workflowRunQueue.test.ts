import { test } from 'node:test';
import assert from 'node:assert/strict';
import { capNote, hasQueued, ordinal, queuePlace } from '../src/modules/workflows/runQueue.ts';

const run = (run_id: string, workflow_name: string, extra: Record<string, unknown> = {}) => ({
  run_id, workflow_id: `wf-${run_id}`, workspace_id: 'ws', workflow_name, status: 'pending' as const,
  started_at: '2026-11-12T10:00:00Z', nodes_total: 0, nodes_done: 0, ...extra,
});

test('ordinals read the way people say them', () => {
  assert.deepEqual([1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 101, 111].map(ordinal),
    ['1st', '2nd', '3rd', '4th', '11th', '12th', '13th', '21st', '22nd', '23rd', '101st', '111th']);
});

test('a queued run says its place and who it waits behind', () => {
  const active = [run('a', 'PR Reviewer'), run('b', 'Deploy'), run('c', 'PR Reviewer — Go')];
  const queue = { max_parallel_runs: 2, queued: [{ run_id: 'a', position: 1 }, { run_id: 'b', position: 2 }, { run_id: 'c', position: 3 }] };
  assert.deepEqual(queuePlace('a', queue, active), {
    chip: 'next in line', line: 'Queued · next in line — starts when a running run finishes',
  });
  assert.deepEqual(queuePlace('c', queue, active), {
    chip: '3rd in line', line: 'Queued · 3rd in line, behind PR Reviewer, Deploy',
  });
});

test('runs ahead in another workspace are counted, not named', () => {
  // The daemon lists only this workspace's queued runs, with GLOBAL positions.
  const queue = { max_parallel_runs: 2, queued: [{ run_id: 'b', position: 2 }, { run_id: 'd', position: 4 }] };
  const active = [run('b', 'Deploy'), run('d', 'PR Reviewer')];
  assert.equal(queuePlace('b', queue, active)?.line, 'Queued · 2nd in line, behind 1 other run');
  assert.equal(queuePlace('d', queue, active)?.line, 'Queued · 4th in line, behind Deploy, 2 other runs');
});

test('a run that is not queued, or no queue view, has no place', () => {
  const queue = { max_parallel_runs: 2, queued: [] };
  assert.equal(queuePlace('a', queue, [run('a', 'X')]), null);
  assert.equal(queuePlace('a', null, [run('a', 'X')]), null);
});

test('the cap note and the fetch gate', () => {
  assert.equal(capNote(2), 'Up to 2 at once — other workflow runs wait in line.');
  assert.equal(hasQueued([run('a', 'X')]), true);
  assert.equal(hasQueued([run('a', 'X', { status: 'running', nodes_done: 1, current_step: 's' })]), false);
  assert.equal(hasQueued([run('a', 'X', { nodes_done: 0, current_step: 'next' })]), false);
});
