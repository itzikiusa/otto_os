// The step pane's tabs by step kind and its phase bar (ux.md §4.4) — runs/stepTabs.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { paneTabs, phaseSegments } from '../src/modules/workflows/runs/stepTabs.ts';
import { at, node, T0 } from './workflowRunFixtures.ts';

test('each kind of step opens on its own first tab', () => {
  assert.deepEqual(paneTabs({ kind: 'agent_prompt' }), ['conversation', 'output', 'files']);
  assert.deepEqual(paneTabs({ kind: 'review_run' }), ['summary', 'summarizer', 'log', 'files']);
  assert.deepEqual(paneTabs({ child: true, kind: 'review_run' }), ['conversation', 'findings', 'files']);
  // A loop's iteration has no findings of its own.
  assert.deepEqual(paneTabs({ child: true, kind: 'loop' }), ['conversation', 'files']);
  assert.deepEqual(paneTabs({ kind: 'script' }), ['log', 'output']);
  assert.deepEqual(paneTabs({ kind: 'human_approval' }), ['preview', 'log']);
  assert.deepEqual(paneTabs({ kind: 'post_review_comments', node: node('a', 'running', 0, null, { attention: { kind: 'approval', text: 'x', since: at(1) } }) }), ['preview', 'log']);
  // An agent step paused on a provider or MCP approval keeps its conversation:
  // the tool call and its context are what a person reads before approving (CR1-040).
  const asks = node('a', 'running', 0, null, { attention: { kind: 'approval', via: 'mcp', text: 'post', since: at(1) }, thread: { thread_id: 't', runner: 'threads', provider: 'claude', turns: 1, usage: {} as never } });
  assert.deepEqual(paneTabs({ kind: 'agent_prompt', node: asks }), ['conversation', 'output', 'files']);
  assert.deepEqual(paneTabs({ kind: 'manual_trigger' }), ['input']);
  assert.deepEqual(paneTabs({ result: true }), ['result', 'files', 'input']);
  assert.deepEqual(paneTabs({ result: true, goal: true }), ['result', 'criteria', 'files', 'input']);
  assert.deepEqual(paneTabs({ kind: 'check' }), ['criteria', 'log', 'output']);
  assert.deepEqual(paneTabs({ kind: 'loop', goal: true }), ['criteria', 'log', 'output']);
  assert.deepEqual(paneTabs({ kind: 'loop' }), ['log', 'output']);
  // A deterministic kind that still ran an agent (a thread) shows its conversation.
  assert.equal(paneTabs({ kind: 'git_checkout', node: node('a', 'success', 0, 1, { thread: { thread_id: 't', runner: 'threads', provider: 'claude', turns: 1, usage: {} as never } }) })[0], 'conversation');
});

test('phase segments run until the next phase, the last until the end', () => {
  const s = phaseSegments([{ phase: 'starting', at: at(0) }, { phase: 'waiting', at: at(0.1), note: 'memory' }, { phase: 'working', at: at(5) }], T0 + 11 * 60_000);
  assert.deepEqual(s.map((x) => [x.label, Math.round(x.ms / 1000)]), [['Starting', 6], ['Waited', 294], ['Working', 360]]);
  assert.equal(s[1].note, 'memory');
});
