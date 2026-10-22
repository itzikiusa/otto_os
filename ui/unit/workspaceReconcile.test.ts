// error-handling-3-03: a by-id fetch that FAILS is not evidence that the open
// tabs / panes it covers are gone — they (and their last known rows) survive
// the refresh. A successful `?ids=` answer that omits a tab still prunes it.
// error-handling-3-01: a failed list load reads as loadErrorText, not raw text.
// ui-correctness-3-06: a workspace switch clears the old workspace's runs.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import * as sessionScope from '../src/lib/stores/sessionScope.ts';
import * as sessionPatch from '../src/lib/stores/sessionPatch.ts';
import * as sessionBuckets from '../src/lib/stores/sessionBuckets.ts';

const row = (id: string, workspace_id = 'w') =>
  ({ id, workspace_id, kind: 'agent', status: 'idle', archived: false, title: id, last_active_at: '2026-10-01', meta: {} });

function fixture(get: (path: string) => Promise<unknown>) {
  const retained: string[][] = [];
  const cleared: (string | null)[] = [];
  const panes: string[] = [];
  const { ws } = loadSource(new URL('../src/lib/stores/workspace.svelte.ts', import.meta.url), {
    '../api/client': { getToken: () => 'token', api: { get } },
    './workflowRuns.svelte': { workflowRuns: { list: [], refresh: async () => {}, apply() {}, clear: (id: string | null = null) => cleared.push(id) } },
    '../api/workspaces': { fetchWorkspace: async () => ({}) },
    '../router.svelte': { router: { mayChangeWorkspace: () => true } }, '../toast.svelte': { toasts: {} }, '../toastError': { toastError() {} },
    '../plural': { plural() {} }, '../confirm.svelte': { confirmer: {} },
    './ui.svelte': { ui: {}, clientId: () => 'test' }, '../win': { winKey: (k: string) => k },
    './splitLayout.svelte': { layout: { panes, focusedIndex: 0, retain: (keep: (t: string) => boolean) => retained.push(panes.filter(keep)), setFocusedSession() {} } },
    './splitLayout': { MAX_PANES: 15, LS_PANES: 'panes' },
    '../storage': { lsGet: () => null, lsSet() {} }, '../desktop': { isEmbedded: false },
    './sessionScope': sessionScope, './sessionPatch': sessionPatch, './sessionBuckets': sessionBuckets,
    '../lazy-component.svelte': { whenIdle: () => () => {} },
  });
  ws.currentId = 'w';
  ws.archivedKnown = true; // no idle archive probe
  // The harness does not run Svelte reactivity: derive the lookup live.
  Object.defineProperty(ws, 'sessionById', { get: () => new Map(ws.sessions.map((s: any) => [s.id, s])) });
  return { ws, retained, cleared, panes };
}

test('a failed by-id chunk keeps its tabs, panes and last known rows', async () => {
  const { ws, panes } = fixture(async (path) => {
    if (path.startsWith('/sessions?ids=')) throw new TypeError('Load failed');
    return [];
  });
  ws.sessions = [row('bg')];
  ws.openTabs = ['bg'];
  panes.push('bg');
  await ws.refreshSessions();
  assert.deepEqual([...ws.openTabs], ['bg']);
  assert.deepEqual([...ws.sessions.map((s: any) => s.id)], ['bg']);
  assert.equal(ws.sessionsError, null);
});

test('a successful by-id answer that omits a tab still prunes the phantom', async () => {
  const { ws, retained } = fixture(async (path) => (path.startsWith('/sessions?ids=') ? [row('live')] : []));
  ws.sessions = [row('gone'), row('live')];
  ws.openTabs = ['gone', 'live'];
  await ws.refreshSessions();
  assert.deepEqual([...ws.openTabs], ['live']);
  assert.ok(retained.length > 0);
});

test('a failed list load shows loadErrorText, not the raw exception', async () => {
  const { ws } = fixture(async (path) => {
    if (path.startsWith('/workspaces/w/')) throw new TypeError('Load failed');
    return [];
  });
  await ws.refreshSessions();
  assert.ok(ws.sessionsError);
  assert.notEqual(ws.sessionsError, 'Load failed');
});

test('switching workspace clears the previous workspace’s workflow runs at once', async () => {
  const { ws, cleared } = fixture(async () => []);
  ws.workspaces = [{ id: 'w2', settings: {} }];
  const switching = ws.select('w2').catch(() => {});
  // Synchronously, before the new workspace's sessions (and runs) load.
  assert.deepEqual(cleared, ['w2']);
  await switching;
});

test('a late runs response for the old workspace is dropped after clear(new id)', async () => {
  let resolve!: (v: unknown) => void;
  const { workflowRuns } = loadSource(new URL('../src/lib/stores/workflowRuns.svelte.ts', import.meta.url), {
    '../api/workflows': { listActiveWorkflowRuns: () => new Promise((r) => { resolve = r; }) },
  });
  const late = workflowRuns.refresh('A');
  workflowRuns.clear('B');
  resolve([{ run_id: 'r1', status: 'running' }]);
  await late;
  assert.equal(workflowRuns.list.length, 0);
});
