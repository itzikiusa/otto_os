// The details card's Subagents section (threadSubagents.ts): subagents only,
// one set of status words, running first and the rest under Previous.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { subagentSection, subagentWord } from '../src/modules/threads/threadSubagents.ts';
import type { AgentTask } from '../src/modules/threads/runtime/agentTasks.ts';

const task = (p: Partial<AgentTask>): AgentTask => ({
  id: 'x', kind: 'agent', label: 'x', status: 'running', background: true, startedAt: '2026-11-23T10:00:00Z', endedAt: null,
  itemId: null, childSessionId: null, taskId: null, stepSummary: null, sublabel: null, prompt: null, output: null, error: null,
  toolUses: 0, detailOmitted: false, ...p,
});

test('background commands and timers are not subagents; status words are the five', () => {
  const s = subagentSection(
    {
      running: [task({ id: 'a', label: 'PR sweep' }), task({ id: 'c', kind: 'command', label: 'sleep 300' }), task({ id: 't', kind: 'thread', label: 'Tests', childSessionId: 'rt_k', status: 'needs-you' })],
      finished: [task({ id: 'd', label: 'Old sweep', status: 'done' }), task({ id: 'tm', kind: 'timer', label: 'in 5m', status: 'done' }), task({ id: 'f', label: 'Broke', status: 'failed' })],
    },
    { providerOf: (id) => (id === 'rt_k' ? 'codex' : null) },
  );
  assert.deepEqual(s.running.map((r) => [r.title, r.word, r.provider]), [['PR sweep', 'Working', null], ['Tests', 'Needs you', 'codex']]);
  assert.deepEqual(s.previous.map((r) => [r.title, r.word]), [['Old sweep', 'Done'], ['Broke', 'Failed']]);
  assert.equal(s.running[1].threadId, 'rt_k');
  assert.equal(s.parent, null);
});

test('delegated terminal threads join once; the parent leads', () => {
  const s = subagentSection(
    { running: [task({ id: 'k', kind: 'thread', label: 'Kid', childSessionId: 'rt_k' })], finished: [] },
    {
      parent: { id: 'rt_p', title: 'Lead' },
      delegated: [
        { id: 'rt_k', title: 'Kid again', provider: 'claude', status: 'working' },
        { id: 's1', title: 'Shell worker', provider: 'claude', status: 'idle' },
      ],
    },
  );
  assert.deepEqual(s.parent, { id: 'rt_p', title: 'Lead' });
  assert.deepEqual(s.running.map((r) => r.title), ['Kid']);
  assert.deepEqual(s.previous.map((r) => [r.title, r.word, r.task]), [['Shell worker', 'Done', null]]);
});

test('raw thread statuses map onto the five words', () => {
  assert.equal(subagentWord('working'), 'Working');
  assert.equal(subagentWord('awaiting_permission'), 'Needs you');
  assert.equal(subagentWord('closed'), 'Stopped');
  assert.equal(subagentWord('idle'), 'Done');
  assert.equal(subagentWord('failed'), 'Failed');
});
