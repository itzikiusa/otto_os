// The Connections hub's SSH / custom-CLI tabs: a dead tab is revived, not
// re-focused; closing a live one asks first (ssh-tab-rules.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const rules = loadSource(new URL('../src/modules/database/ssh-tab-rules.ts', import.meta.url), {});

const tabs = [
  { connId: 'a', sessionId: 's-a' },
  { connId: 'b', sessionId: 's-b' },
];
const states: Record<string, 'live' | 'exited'> = { 's-a': 'live', 's-b': 'exited' };

test('opening a connection: new tab, focus a live one, reconnect an ended one', () => {
  const of = (s: string) => states[s];
  assert.equal(rules.sshOpenAction(tabs, of, 'z'), 'open');
  assert.equal(rules.sshOpenAction(tabs, of, 'a'), 'focus');
  assert.equal(rules.sshOpenAction(tabs, of, 'b'), 'reconnect');
  // Not reported yet (the pane has not attached): treat it as live.
  assert.equal(rules.sshOpenAction(tabs, () => undefined, 'b'), 'focus');
});

test('only an exited status ends a tab; a dormant (reconnectable) shell resumes in place', () => {
  assert.equal(rules.livenessOf('exited'), 'exited');
  assert.equal(rules.livenessOf('reconnectable'), 'live');
  assert.equal(rules.livenessOf('running'), 'live');
  assert.equal(rules.livenessOf('idle'), 'live');
});

test('closing asks while the session runs, names the connection, and not once it ended', () => {
  assert.equal(
    rules.sshCloseQuestion({ name: 'prod-bastion', kind: 'ssh' }, 'live'),
    'Close the SSH session to prod-bastion? Anything running in it stops.',
  );
  assert.equal(
    rules.sshCloseQuestion({ name: 'psql', kind: 'custom' }, undefined),
    'Close the terminal session to psql? Anything running in it stops.',
  );
  assert.equal(rules.sshCloseQuestion({ name: 'prod-bastion', kind: 'ssh' }, 'exited'), null);
});

test('Reconnect swaps the session id in place, keeping the tab order', () => {
  const next = rules.withSession(tabs, 'a', 's-a2');
  assert.deepEqual(next.map((t: { connId: string; sessionId: string }) => `${t.connId}:${t.sessionId}`), ['a:s-a2', 'b:s-b']);
  assert.equal(tabs[0].sessionId, 's-a', 'the input list is not mutated');
});
