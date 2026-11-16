// Every subagent, child thread, and background task of a runtime thread (agentTasks.ts):
// the strip above the composer and Thread details' "Agents & tasks" drill-down.
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

test('drill-down surfaces live subagent child steps, streaming command output, child threads, and settles expired timers', () => {
  const items = [
    card('t1:sa:1', 'IDE Backend Fixer', 'running', '2026-11-13T10:00:00Z', {
      subagent_type: 'self',
      prompt: 'Fix gopls build tags',
      tool_uses: 3,
      last_tool: 'edit_file',
    }),
    {
      ...base,
      id: 't1:cmd:1',
      ordinal: 2,
      status: 'running',
      started_at: '2026-11-13T10:00:02Z',
      type: 'command_execution',
      input: 'cargo nextest run -p otto-ide',
      cwd: '/Users/dev/claude_ade-ide',
      output: '',
    } as never,
    {
      ...base,
      id: 't2:msg:1',
      turn_id: 't2',
      ordinal: 3,
      status: 'completed',
      started_at: '2026-11-13T10:02:00Z',
      updated_at: '2026-11-13T10:02:00Z',
      type: 'user_message',
      text: 'Status check',
    } as never,
  ];
  const background = [
    bg('sa-1', 'IDE Backend Fixer', '2026-11-13T10:00:00Z', 't1:sa:1'),
    { id: 'task-9', kind: 'command', label: 'cargo nextest run -p otto-ide', started_at: '2026-11-13T10:00:02Z', item_id: 't1:cmd:1' },
    { id: 'sched-1', kind: 'command', label: 'Timer (35s): Verify Post-Round-3 deploy receipt', started_at: '2026-11-13T10:00:05Z' },
  ];
  const t = agentTasks(items, background, {
    output: { 't1:cmd:1': 'Compiling otto-ide v0.1.0\nPASS [ 0.42s] pool_tests::warm_reuse\n' },
    childThreads: [
      {
        id: 'rt_child_1',
        title: 'Security Reviewer',
        provider: 'codex',
        model: 'gpt-5.4',
        status: 'working',
        createdAt: '2026-11-13T10:00:10Z',
        updatedAt: '2026-11-13T10:01:00Z',
        stepSummary: 'Read 6 files',
      },
    ],
  });

  // One-shot 35s timer started at 10:00:05Z with activity at 10:02:00Z settles to finished!
  assert.deepEqual(t.running.map((r) => r.label), [
    'IDE Backend Fixer',
    'cargo nextest run -p otto-ide',
    'Security Reviewer',
  ]);
  assert.equal(t.running[0].sublabel, 'self');
  assert.equal(t.running[0].stepSummary, '3 steps · last: edit_file');
  assert.equal(t.running[0].prompt, 'Fix gopls build tags');
  assert.equal(t.running[1].stepSummary, 'PASS [ 0.42s] pool_tests::warm_reuse');
  assert.equal(t.running[1].output, 'Compiling otto-ide v0.1.0\nPASS [ 0.42s] pool_tests::warm_reuse\n');
  assert.equal(t.running[2].childSessionId, 'rt_child_1');
  assert.equal(t.running[2].sublabel, 'codex · gpt-5.4');
  assert.equal(t.running[2].stepSummary, 'Read 6 files');
  assert.equal(backgroundStripText([t.running[0]]), 'Running in the background: IDE Backend Fixer (3 steps · last: edit_file)');

  assert.equal(t.finished.length, 1);
  assert.equal(t.finished[0].kind, 'timer');
  assert.equal(t.finished[0].status, 'done');
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
    [running.running[0].itemId, running.running[0].taskId, running.running[0].prompt],
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

test('structured task_settlement, due_at deadlines, and top-level dynamic_tool subagents work across all providers', () => {
  const items = [
    {
      ...base,
      id: 't1:dyn:agy1',
      ordinal: 1,
      status: 'running',
      started_at: '2026-11-13T10:00:00Z',
      type: 'dynamic_tool',
      tool_name: 'invoke_subagent',
      input: { Role: 'Architecture Reviewer', TypeName: 'research', Prompt: 'Audit layer boundaries' },
    } as never,
    {
      ...base,
      id: 't1:dyn:done1',
      ordinal: 2,
      status: 'completed',
      started_at: '2026-11-13T10:00:01Z',
      completed_at: '2026-11-13T10:00:12Z',
      type: 'dynamic_tool',
      tool_name: 'otto_personal_agent_delegate',
      input: { agent: 'qa-bot', task: 'Run smoke checks' },
      output: 'All 14 smoke checks passed',
    } as never,
    {
      ...base,
      id: 'notice:settle:1',
      ordinal: 3,
      status: 'completed',
      started_at: null,
      updated_at: '2026-11-13T10:01:30Z',
      type: 'system_notice',
      message: 'Custom provider notice text',
      task_settlement: {
        task_id: 'cmd-42',
        label: 'cargo test -p otto-ide',
        kind: 'command',
        status: 'failed',
      },
    } as never,
  ];

  const background = [
    {
      id: 'timer-due-1',
      kind: 'timer',
      label: 'Check deploy health',
      started_at: '2026-11-13T10:00:05Z',
      due_at: '2026-11-13T10:01:00Z',
    },
  ];

  const t = agentTasks(items, background);
  assert.equal(t.running.length, 1);
  assert.equal(t.running[0].label, 'Architecture Reviewer');
  assert.equal(t.running[0].sublabel, 'invoke_subagent');
  assert.equal(t.running[0].prompt, 'Audit layer boundaries');

  // Structured task_settlement + due_at-expired timer + completed dynamic_tool subagent all settle cleanly!
  assert.deepEqual(
    t.finished.map((f) => [f.label, f.kind, f.status]),
    [
      ['cargo test -p otto-ide', 'command', 'failed'],
      ['Check deploy health', 'timer', 'done'],
      ['qa-bot', 'agent', 'done'],
    ],
  );
  assert.equal(t.finished[2].output, 'All 14 smoke checks passed');
});

test('a swarm row and a swarm subagent card both name their agy member; other rows none', async () => {
  const { swarmMemberOf, withSwarm } = await import('../src/modules/threads/runtime/agentTasks.ts');
  const carded = card('t1:swarm:echoer', 'echoer', 'running', '2026-11-13T10:00:00Z', { tool_name: 'swarm' });
  const plain = card('t1:tool:a', 'alpha', 'running', '2026-11-13T10:00:00Z');
  const items = [carded, plain];
  const t = withSwarm(agentTasks(items, []), [{ name: 'echoer', killed: false }, { name: 'dap-lsp', killed: false }], items);
  const by = (label: string) => t.running.find((r) => r.label === label)!;
  assert.equal(swarmMemberOf(by('echoer'), items), 'echoer');
  assert.equal(swarmMemberOf(by('dap-lsp'), items), 'dap-lsp');
  assert.equal(swarmMemberOf(by('alpha'), items), null);
});
