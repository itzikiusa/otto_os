// A runtime thread that finishes a turn while nobody looks gets the sidebar's
// unread dot; one on screen (focused in Threads, or shown by an embed) does
// not. An embed reads only its own row.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function load(router: { module?: string }, threads: { focusedSessionId?: string | null }, get = async (_p: string): Promise<unknown> => []) {
  const gets: string[] = [];
  const { runtimeThreads } = loadSource(new URL('../src/modules/threads/runtime/runtimeThreads.svelte.ts', import.meta.url), {
    '../../../lib/api/client': { api: { get: (p: string) => { gets.push(p); return get(p); } } },
    '../../../lib/stores/runtimeDraftRecovery.svelte': { installRuntimeDraftRecovery: () => {} },
    '../../../lib/live': { onLive: () => () => {} }, '../../../lib/router.svelte': { router },
    '../../../lib/stores/runtimeAttention.svelte': { noteRuntimeWaiting: () => {} },
    '../../../lib/stores/uiControl.svelte': { uiControl: { addSource: () => {} } },
    '../threadsStore.svelte': { threads }, '../../../lib/stores/runtimeDrafts.svelte': { runtimeDrafts: { forget: () => {} } },
  });
  return { runtimeThreads, gets };
}

const row = { id: 'A', workspace_id: 'ws', status: 'idle', title: 'A', last_active_at: '2026-11-12T10:00:00Z' };
const done = (id: string, status = 'completed') => ({ session_id: id, sequence: 9, ts: 'now', type: 'turn.terminal', turn_id: 't', status });

test('a finished turn off screen is unread until marked read', () => {
  const { runtimeThreads } = load({ module: 'agents' }, { focusedSessionId: 'A' });
  runtimeThreads.byId = { A: row };
  runtimeThreads.onEvent(done('A'));
  assert.equal(runtimeThreads.unread.A, true);
  runtimeThreads.markRead('A');
  assert.equal(runtimeThreads.unread.A, undefined);
  // A turn the person stopped is not news.
  runtimeThreads.onEvent(done('A', 'interrupted'));
  assert.equal(runtimeThreads.unread.A, undefined);
});

test('focused in Threads, or shown by an embed, a finished turn is seen', () => {
  const { runtimeThreads } = load({ module: 'threads' }, { focusedSessionId: 'A' });
  runtimeThreads.byId = { A: row, B: { ...row, id: 'B' } };
  runtimeThreads.onEvent(done('A'));
  assert.equal(runtimeThreads.unread.A, undefined);
  const release = runtimeThreads.watch('B');
  runtimeThreads.onEvent(done('B'));
  assert.equal(runtimeThreads.unread.B, undefined);
  release();
  runtimeThreads.onEvent(done('B'));
  assert.equal(runtimeThreads.unread.B, true);
});

test('an embed loads its own row by id, not the workspace list', async () => {
  const { runtimeThreads, gets } = load({}, {}, async () => row);
  await runtimeThreads.loadOne('A');
  assert.deepEqual(gets, ['/runtime-sessions/A']);
  assert.equal(runtimeThreads.byId.A.id, 'A');
});
