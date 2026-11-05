import { test } from 'node:test';
import assert from 'node:assert/strict';
import { agentThreadBadge, personalAgentName, personalAgentOf, routineLabel, runBadge } from '../src/lib/personalAgentBadges.ts';

const pa = (extra: Record<string, unknown>) => ({ source: 'personal_agent', personal_agent: 'A1', ...extra });

test('a plain thread is not an agent thread', () => {
  assert.equal(personalAgentOf({ surface: 'threads' }), null);
  assert.equal(agentThreadBadge(null), null);
  assert.equal(personalAgentName({}, 'Chat: Scout'), null);
});

test('the agent name comes from the meta, else the daemon-given title', () => {
  assert.equal(personalAgentName(pa({ personal_agent_name: 'Scout' }), 'whatever'), 'Scout');
  // Older threads carry no name: "Chat: X" / "Agent: X" titles still name it.
  assert.equal(personalAgentName(pa({}), 'Chat: Bitbucket helper'), 'Bitbucket helper');
  assert.equal(personalAgentName(pa({}), 'Agent: Daily Recap'), 'Daily Recap');
  assert.equal(personalAgentName(pa({}), 'renamed thread'), null);
});

test('every trigger gets its own badge', () => {
  assert.equal(agentThreadBadge(pa({ personal_agent_chat: true }))?.label, 'Chat');
  assert.equal(agentThreadBadge(pa({ run_id: 'r', trigger: 'schedule', routine: 'Morning brief' }))?.label, 'Scheduled · Morning brief');
  // A run from before `trigger` was stored: its mode still says it.
  assert.equal(agentThreadBadge(pa({ run_id: 'r', agent_mode: 'scheduled' }))?.label, 'Scheduled');
  assert.equal(agentThreadBadge(pa({ run_id: 'r', trigger: 'manual', agent_mode: 'directed' }))?.label, 'Manual run');
  assert.equal(agentThreadBadge(pa({ run_id: 'r', trigger: 'proactive' }))?.kind, 'proactive');
  assert.equal(agentThreadBadge(pa({ delegated_by_agent: 'L', delegated_by_name: 'Otto' }))?.label, 'Delegated by Otto');
  assert.equal(agentThreadBadge(pa({ delegated_by_agent: 'L' }), (id) => (id === 'L' ? 'Main' : undefined))?.label, 'Delegated by Main');
});

test('run rows and routine names match the daemon', () => {
  assert.equal(runBadge({ trigger: 'schedule' }, 'Brief').label, 'Scheduled · Brief');
  assert.equal(runBadge({ trigger: 'manual', mode: 'directed' }).label, 'Manual run');
  assert.equal(runBadge({ trigger: 'proactive', mode: 'proactive' }).label, 'Proactive');
  assert.equal(routineLabel('# Morning brief\nsummarise'), 'Morning brief');
  assert.equal(routineLabel('  '), 'Routine');
  assert.ok(routineLabel('x'.repeat(80)).endsWith('…'));
});
