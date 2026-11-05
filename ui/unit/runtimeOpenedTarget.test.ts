// The quick link under an Otto thread/session tool call in a structured
// thread (modules/threads/runtime/timeline.ts `openedTarget`).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { openedTarget } from '../src/modules/threads/runtime/timeline.ts';
import type { TurnItem } from '../src/lib/api/types';

const tool = (tool_name: string, input: unknown, output?: string, error?: string): TurnItem =>
  ({ id: 'i', turn_id: 't', status: 'completed', type: 'dynamic_tool', tool_name, input, output, error }) as unknown as TurnItem;
const THREAD = '01M6N0NRBX997VVJ2TRVA9N97V';
const SESSION = '01M6N0N2WWTH9T8X3GCA3VE7TP';

test('thread_open links to the thread it opened', () => {
  const out = JSON.stringify({ id: THREAD, provider: 'codex', access_mode: 'full-access' });
  assert.deepEqual(openedTarget(tool('mcp__otto__otto_thread_open', {}, out)), { kind: 'thread', id: THREAD });
});

test('open_session links to a terminal session, or to a thread when it opened one', () => {
  const term = JSON.stringify({ id: SESSION, kind: 'agent', provider: 'codex' });
  assert.deepEqual(openedTarget(tool('mcp__otto__otto_open_session', {}, term)), { kind: 'session', id: SESSION });
  const thread = JSON.stringify({ id: THREAD, access_mode: 'full-access' });
  assert.deepEqual(openedTarget(tool('otto.open_session', {}, thread)), { kind: 'thread', id: THREAD });
});

test('message / read link to the peer; failures and other tools have no link', () => {
  assert.deepEqual(openedTarget(tool('mcp__otto__otto_thread_message', { to_session_id: THREAD, text: 'x' })), { kind: 'thread', id: THREAD });
  assert.deepEqual(openedTarget(tool('mcp__otto__otto_thread_read', { peer_session_id: THREAD })), { kind: 'thread', id: THREAD });
  assert.equal(openedTarget(tool('mcp__otto__otto_thread_open', {}, undefined, 'boom')), null);
  assert.equal(openedTarget(tool('mcp__otto__otto_list_sessions', {}, `{"id":"${SESSION}"}`)), null);
});
