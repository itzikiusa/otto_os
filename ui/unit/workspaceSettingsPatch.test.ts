import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import * as sessionScope from '../src/lib/stores/sessionScope.ts';
import * as sessionPatch from '../src/lib/stores/sessionPatch.ts';
import * as sessionBuckets from '../src/lib/stores/sessionBuckets.ts';

function fixture() {
  const writes: any[] = [];
  const { ws } = loadSource(new URL('../src/lib/stores/workspace.svelte.ts', import.meta.url), {
    '../api/client': { getToken: () => 'token', api: { patch: async (path: string, body: unknown) => {
      writes.push({ path, body }); return { id: 'w', settings: {} };
    } } },
    './workflowRuns.svelte': { workflowRuns: { list: [], refresh: async () => {}, apply() {}, clear() {} } },
    '../api/workspaces': { fetchWorkspace: async () => ({ settings: { notes: 'old' } }) },
    '../router.svelte': { router: {} }, '../toast.svelte': { toasts: {} }, '../toastError': { toastError() {} },
    '../plural': { plural() {} }, '../confirm.svelte': { confirmer: {} },
    './ui.svelte': { ui: {}, clientId: () => 'test' }, '../win': { winKey: (k: string) => k },
    './splitLayout.svelte': { layout: { panes: [], focusedIndex: 0 } }, './splitLayout': { MAX_PANES: 15, LS_PANES: 'panes' },
    '../storage': { lsGet: () => null, lsSet() {} }, '../desktop': { isEmbedded: false },
    './sessionScope': sessionScope, './sessionPatch': sessionPatch, './sessionBuckets': sessionBuckets,
    '../lazy-component.svelte': { whenIdle: () => () => {} },
  });
  ws.currentId = 'w';
  ws.workspaces = [{ id: 'w', settings: { notes: 'old', api_client: { allow_local: false, history_max_rows: 100 } } }];
  // The source harness deliberately does not run Svelte reactivity.
  Object.defineProperty(ws, 'current', { get: () => ws.workspaces.find((w: any) => w.id === ws.currentId) });
  return { ws, writes };
}

test('every workspace preference sends only an atomic key patch, never cached settings', async () => {
  const { ws, writes } = fixture();
  await ws.saveNotes('new');
  await ws.saveDefaultAgent('codex');
  // The api-client keys moved to the apiClient store (unit/apiWsSettings.test.ts).
  assert.equal(ws.setApiAllowLocal, undefined);
  await ws.patchSettings('w', { api_client: { automation_runs_keep: 5 } });
  assert.deepEqual(JSON.parse(JSON.stringify(writes.map(w => w.body))), [
    { settings_patch: { notes: 'new' } },
    { settings_patch: { default_provider: 'codex' } },
    { settings_patch: { api_client: { automation_runs_keep: 5 } } },
  ]);
});
