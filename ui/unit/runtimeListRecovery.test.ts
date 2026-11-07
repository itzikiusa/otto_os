import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

test('failed runtime list retains rows, exposes retryable error, and clears it after recovery', async () => {
  let fails = true;
  const oldRow = { id: 'old', workspace_id: 'ws', status: 'idle' };
  const nextRow = { id: 'next', workspace_id: 'ws', status: 'idle' };
  const { runtimeThreads } = loadSource(new URL('../src/modules/threads/runtime/runtimeThreads.svelte.ts', import.meta.url), {
    '../../../lib/api/client': { api: { get: async () => { if (fails) throw new Error('offline'); return [nextRow]; } } },
    '../../../lib/stores/runtimeDraftRecovery.svelte': { installRuntimeDraftRecovery: () => {} },
    '../../../lib/live': { onLive: () => () => {} }, '../../../lib/router.svelte': { router: {} },
    '../../../lib/stores/runtimeAttention.svelte': { noteRuntimeWaiting: () => {} },
    '../../../lib/stores/uiControl.svelte': { uiControl: { addSource: () => {} } },
    '../threadsStore.svelte': { threads: {} }, '../../../lib/stores/runtimeDrafts.svelte': { runtimeDrafts: { forget: () => {} } },
  });
  runtimeThreads.byId = { old: oldRow };
  await assert.rejects(runtimeThreads.load('ws'), /offline/);
  assert.equal(runtimeThreads.byId.old, oldRow);
  assert.equal(runtimeThreads.loaded.ws, true);
  assert.ok(runtimeThreads.loadErrors?.ws);
  fails = false;
  await runtimeThreads.load('ws');
  assert.equal(runtimeThreads.loadErrors.ws, undefined);
  assert.equal(runtimeThreads.byId.old, undefined);
  assert.equal(runtimeThreads.byId.next.id, 'next');
});
