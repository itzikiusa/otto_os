// The PR agent bridge (pr-agent-bridge.svelte.ts): an ask waits for its own
// PR's panel, and the thread's live state feeds the ⋯ entry's hint.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const { prAgentBridge: b } = loadSource(new URL('../src/modules/git/pr-agent-bridge.svelte.ts', import.meta.url), {}, {}, { relative: true });

test('an ask is taken only by its own PR, once', () => {
  b.ask('rep', 5, { kind: 'explain' });
  assert.equal(b.take('rep#6'), null);
  assert.deepEqual(JSON.parse(JSON.stringify(b.take('rep#5'))), { kind: 'explain' });
  assert.equal(b.take('rep#5'), null);
});

test('live state shows on the ⋯ hint and clears when the panel goes', () => {
  assert.equal(b.hint('rep', 5), 'agent');
  b.setStatus('rep#5', 'working');
  assert.equal(b.hint('rep', 5), 'agent · working');
  assert.equal(b.hint('rep', 6), 'agent');
  b.setStatus('rep#5', 'needs-you');
  assert.equal(b.hint('rep', 5), 'agent · needs you');
  b.setStatus('rep#5', null);
  assert.equal(b.hint('rep', 5), 'agent');
});

test('detach / reattach', () => {
  b.detach('rep#5', 't1');
  assert.equal(b.detached['rep#5'], 't1');
  b.reattach('rep#5');
  assert.equal(b.detached['rep#5'], undefined);
});
