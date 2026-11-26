// Node cards' second line and icon (modules/workflows/editor/nodeSummary.ts):
// one case per family, Default spelled out for an empty model or effort.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { nodeIcon, nodeSummary, modelChip, AGENT_KINDS } from '../src/modules/workflows/editor/nodeSummary.ts';

const n = (kind: string, params: Record<string, unknown> = {}, id = 'n', name = id) => ({ id, kind, name, x: 0, y: 0, params });

test('agent steps: provider · model · effort, Default for what is empty, and the step whose thread they continue', () => {
  assert.equal(nodeSummary(n('agent_prompt', { provider: 'claude', effort: 'high' })), 'Claude · Default · High');
  assert.equal(nodeSummary(n('agent_prompt', {}), { defaultProvider: 'codex' }), 'Codex · Default · Default');
  assert.equal(nodeSummary(n('agent_prompt', { provider: 'codex', model: 'gpt-5.1-codex', effort: 'xhigh' })), 'Codex · gpt-5.1-codex · Extra high');
  const nodes = [n('agent_prompt', {}, 'co', 'Check out PR')];
  assert.equal(nodeSummary(n('agent_prompt', { provider: 'claude', continue_from: 'co' }), { nodes }), 'Claude · Default · Default · continues Check out PR');
  assert.equal(modelChip({}, ''), 'Default · Default · Default');
  assert.equal(nodeIcon('agent_prompt'), 'assistant');
  assert.equal(nodeIcon('git_pr'), 'assistant');
  assert.ok(!AGENT_KINDS.includes('condition'), 'a condition has no provider');
});

test('the trigger names how runs start and its declared inputs', () => {
  assert.equal(nodeSummary(n('manual_trigger', { inputs: [{ key: 'pr' }, { key: 'base' }, { key: 'goals' }, { key: 'msg' }] }), { triggerKinds: ['chat'] }), 'Chat or manual · 4 inputs');
  assert.equal(nodeSummary(n('manual_trigger')), 'Manual');
  assert.equal(nodeIcon('manual_trigger'), 'play');
});

test('review: lenses × providers and the pass mark', () => {
  const reviewers = [
    { lens: 'a', providers: ['claude', 'codex'] }, { lens: 'b', providers: ['claude'] }, { lens: 'c', providers: ['codex'] },
    { lens: 'd' }, { lens: 'e' }, { lens: 'f' },
  ];
  assert.equal(nodeSummary(n('review_run', { reviewers, threshold: 80 })), '6 lenses × Claude, Codex · pass ≥ 80');
  assert.equal(nodeSummary(n('review_run')), 'Default lenses · pass ≥ 80');
  assert.equal(nodeIcon('review_run'), 'eye');
});

test('deterministic steps say what they do', () => {
  assert.equal(nodeSummary(n('git_checkout', {})), 'PR from input, then branch');
  assert.equal(nodeSummary(n('git_checkout', { mode: 'branch' })), 'Branch from input, then base');
  assert.equal(nodeSummary(n('script', { command: 'bash ~/skills/go/pr_gates.sh "$OTTO_WORKTREE"', timeout_s: 900 })), 'Runs pr_gates.sh · 15 min limit');
  assert.equal(nodeSummary(n('script', {})), 'No command yet');
  assert.equal(nodeSummary(n('post_review_comments', {})), 'Bug and warn inline · notes combined · summary');
  assert.equal(nodeSummary(n('post_review_comments', { inline: [], info: 'skip', summary: false })), 'Nothing inline');
  assert.equal(nodeSummary(n('check', { criteria: [{ verify: 'command' }, { verify: 'human' }, { verify: 'agent' }] })), '3 criteria · 1 by you');
  assert.deepEqual([nodeIcon('git_checkout'), nodeIcon('script'), nodeIcon('post_review_comments'), nodeIcon('check')], ['branch', 'command', 'comment', 'target']);
});

test('flow and I/O steps', () => {
  assert.equal(nodeSummary(n('condition', { expr: 'repos.count > 1' })), 'repos.count > 1');
  assert.equal(nodeSummary(n('loop', { max_iterations: 3, until: 'passed' })), 'Up to 3 times · until passed');
  assert.equal(nodeSummary(n('delay', { ms: 300_000 })), 'Waits 5m');
  assert.equal(nodeSummary(n('human_approval', { message: 'Publish to Confluence?' })), 'Asks: Publish to Confluence?');
  assert.equal(nodeSummary(n('http_request', { method: 'post', url: 'https://api.example.com/x' })), 'POST api.example.com');
  assert.equal(nodeSummary(n('db_query', { statement: 'SELECT  id\nFROM orders' })), 'SELECT id FROM orders');
  assert.equal(nodeSummary(n('channel_notify', { channel: 'slack', message: 'Score {score}' })), 'slack · Score {score}');
  assert.equal(nodeIcon('budget_gate'), 'gauge');
  assert.equal(nodeIcon('some_future_kind'), 'box');
});
