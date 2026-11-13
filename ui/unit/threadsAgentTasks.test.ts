// Every subagent and background task of a runtime thread (agentTasks.ts):
// the strip above the composer and Thread details' "Agents & tasks".
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { agentTasks, backgroundStripText, taskElapsed } from '../src/modules/threads/runtime/agentTasks.ts';

const base = { turn_id: 't1', title: null, completed_at: null, updated_at: '2026-11-13T10:00:30Z' };
const card = (id: string, description: string, status: string, startedAt: string, extra: Record<string, unknown> = {}) =>
  ({ ...base, id, ordinal: 1, status, started_at: startedAt, type: 'subagent', tool_name: 'Agent', description, background: true, tool_uses: 0, ...extra }) as never;
const bg = (id: string, label: string, started_at: string, item_id?: string) => ({ id, kind: 'agent', label, started_at, item_id });

test('two concurrent background agents both run, and the strip counts them', () => {
  const items = [card('t1:tool:a', 'alpha probe', 'running', '2026-11-13T10:00:00Z'), card('t1:tool:b', 'beta probe', 'running', '2026-11-13T10:00:01Z')];
  const t = agentTasks(items, [bg('ta', 'alpha probe', '2026-11-13T10:00:00Z', 't1:tool:a'), bg('tb', 'beta probe', '2026-11-13T10:00:01Z', 't1:tool:b')]);
  assert.deepEqual(t.running.map((r) => r.label), ['alpha probe', 'beta probe']);
  assert.equal(t.finished.length, 0);
  assert.equal(backgroundStripText(t.running), '2 running in the background');
  assert.equal(backgroundStripText(t.running.slice(0, 1)), 'Running in the background: alpha probe');
});

test('a background card still running counts even when the session list lost its task', () => {
  const items = [card('t1:tool:a', 'alpha probe', 'running', '2026-11-13T10:00:00Z'), card('t1:tool:b', 'beta probe', 'running', '2026-11-13T10:00:01Z')];
  const t = agentTasks(items, [bg('ta', 'alpha probe', '2026-11-13T10:00:00Z', 't1:tool:a')]);
  assert.deepEqual(t.running.map((r) => [r.label, r.background]), [['alpha probe', true], ['beta probe', true]]);
});

test('finished work is newest first: settled cards and background command notices', () => {
  const items = [
    card('t1:tool:a', 'alpha probe', 'completed', '2026-11-13T10:00:00Z', { completed_at: '2026-11-13T10:00:05Z' }),
    card('t1:tool:f', 'foreground', 'failed', '2026-11-13T10:00:00Z', { completed_at: '2026-11-13T10:00:09Z', background: false }),
    { ...base, id: 'bg:b7', ordinal: 3, status: 'completed', started_at: null, updated_at: '2026-11-13T10:00:07Z', type: 'system_notice', message: 'Background command finished: npm test' } as never,
    { ...base, id: 'n2', ordinal: 4, status: 'completed', started_at: null, type: 'system_notice', message: 'Something else' } as never,
  ];
  const t = agentTasks(items, []);
  assert.equal(t.running.length, 0);
  assert.deepEqual(t.finished.map((r) => [r.label, r.status, r.kind]), [
    ['foreground', 'failed', 'agent'],
    ['npm test', 'done', 'command'],
    ['alpha probe', 'done', 'agent'],
  ]);
});

test('elapsed reads seconds, minutes and hours, live while running', () => {
  const now = Date.parse('2026-11-13T10:02:05Z');
  assert.equal(taskElapsed({ startedAt: '2026-11-13T10:00:00Z', endedAt: null }, now), '2m 5s');
  assert.equal(taskElapsed({ startedAt: '2026-11-13T10:00:00Z', endedAt: '2026-11-13T10:00:09Z' }, now), '9s');
  assert.equal(taskElapsed({ startedAt: '2026-11-13T08:00:00Z', endedAt: '2026-11-13T09:05:00Z' }, now), '1h 5m');
  assert.equal(taskElapsed({ startedAt: null, endedAt: null }, now), null);
});

test('a background command opens its launch call and output, running or finished', () => {
  const launch = {
    ...base, id: 't1:tool:L', ordinal: 2, status: 'completed', started_at: '2026-11-13T10:00:00Z', type: 'command_execution',
    input: 'for i in 1 2 3; do echo tick $i; sleep 2; done',
    output: 'Command running in background with ID: bbplg7ake. Output is being written to: /private/tmp/claude-501/x/s/tasks/bbplg7ake.output.',
  } as never;
  const running = agentTasks([launch], [{ id: 'bbplg7ake', kind: 'command', label: 'Run three-tick loop', started_at: '2026-11-13T10:00:00Z' }]);
  assert.deepEqual(
    [running.running[0].itemId, running.running[0].taskId, running.running[0].command],
    ['t1:tool:L', 'bbplg7ake', 'for i in 1 2 3; do echo tick $i; sleep 2; done'],
  );
  const notice = { ...base, id: 'bg:bbplg7ake', ordinal: 3, status: 'completed', started_at: null, type: 'system_notice', message: 'Background command finished: Run three-tick loop' } as never;
  const done = agentTasks([launch, notice], []).finished[0];
  assert.deepEqual([done.label, done.itemId, done.taskId, done.startedAt], ['Run three-tick loop', 't1:tool:L', 'bbplg7ake', '2026-11-13T10:00:00Z']);
  // A subagent opens its own card; it has no output file.
  const sub = agentTasks([card('t1:tool:a', 'alpha', 'running', '2026-11-13T10:00:00Z')], []).running[0];
  assert.deepEqual([sub.itemId, sub.taskId], ['t1:tool:a', null]);
});

test('swarm members without a card join the list; carded ones are not repeated', async () => {
  const { withSwarm } = await import('../src/modules/threads/runtime/agentTasks.ts');
  const carded = card('t1:swarm:echoer', 'echoer', 'running', '2026-11-13T10:00:00Z', { tool_name: 'swarm' });
  const t = withSwarm(agentTasks([carded], []), [
    { name: 'echoer', killed: false },
    { name: 'dap-lsp', killed: false, phase: 'RUNNING' },
    { name: 'monitor', killed: true },
  ], [carded]);
  assert.deepEqual(t.running.map((r) => r.label), ['echoer', 'dap-lsp']);
  assert.deepEqual(t.finished.map((r) => [r.label, r.status, r.id]), [['monitor', 'stopped', 'swarm:monitor']]);
});
