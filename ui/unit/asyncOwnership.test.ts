import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

function workspace() {
  const requests: { path: string; result: ReturnType<typeof deferred<any[]>> }[] = [];
  const restored: string[] = [];
  const layout = { panes: [], focusedIndex: 0, bindKey() {}, restore: (id: string) => restored.push(id), retain() {} };
  const api = { get: (path: string) => {
    if (path.includes('/scratch/')) return Promise.resolve([]);
    const result = deferred<any[]>(); requests.push({ path, result }); return result.promise;
  } };
  const { ws } = loadSource(new URL('../src/lib/stores/workspace.svelte.ts', import.meta.url), {
    '../api/client': { api }, '../api/workflows': { listActiveWorkflowRuns: async () => [] }, '../api/workspaces': { fetchWorkspace: async () => ({}) },
    '../router.svelte': { router: {} }, '../toast.svelte': { toasts: {} }, '../confirm.svelte': { confirmer: {} },
    './ui.svelte': { ui: { sessionIsolation: false }, clientId: () => 'test' },
    '../win': { winKey: (key: string) => key }, './splitLayout.svelte': { layout }, './splitLayout': { MAX_PANES: 15 },
    '../storage': { lsGet: () => null, lsSet() {}, lsRemove() {} },
  });
  ws.refreshOtherSessions = async () => {};
  return { ws, requests, restored };
}

test('late workspace selection cannot publish sessions or restore the old layout', async () => {
  const { ws, requests, restored } = workspace();
  const a = ws.select('A'); const b = ws.select('B');
  requests[1].result.resolve([{ id: 'B-session' }]); await b;
  requests[0].result.resolve([{ id: 'A-session' }]); await a;
  assert.equal(ws.currentId, 'B');
  assert.equal(ws.sessions[0].id, 'B-session');
  assert.deepEqual(restored, ['B']);
});

test('older refresh cannot overwrite a newer refresh in the same workspace', async () => {
  const { ws, requests } = workspace();
  ws.currentId = 'A';
  const older = ws.refreshSessions(); const newer = ws.refreshSessions();
  requests[1].result.resolve([{ id: 'new' }]); await newer;
  requests[0].result.resolve([{ id: 'old' }]); await older;
  assert.equal(ws.sessions[0].id, 'new');
});

test('selection waits for a superseding refresh before restoring saved tabs', async () => {
  const { ws, requests, restored } = workspace();
  const selecting = ws.select('A');
  const refreshing = ws.refreshSessions();
  requests[0].result.resolve([{ id: 'obsolete' }]);
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(restored, [], 'no layout restore against an obsolete or empty session list');
  requests[1].result.resolve([{ id: 'latest' }]);
  await Promise.all([selecting, refreshing]);
  assert.equal(ws.sessions[0].id, 'latest');
  assert.deepEqual(restored, ['A']);
});

test('old History pagination cannot append to a new provider list', async () => {
  const page = deferred<{ entries: any[]; next_cursor: string | null }>();
  let calls = 0;
  const rows = Array.from({ length: 100 }, (_, i) => ({ session_id: `claude-${i}`, provider: 'claude', last_active_at: String(i) }));
  const api = { get: async () => ++calls === 1 ? { entries: rows, next_cursor: 'more' } : calls === 2 ? page.promise : { entries: [{ session_id: 'codex', provider: 'codex' }], next_cursor: null } };
  const { history } = loadSource(new URL('../src/modules/agents/history/history.svelte.ts', import.meta.url), {
    '../../../lib/api/client': { api }, '../../../lib/stores/activity.svelte': { activity: {} },
  });
  await history.load('A');
  const loading = history.loadMore();
  assert.equal(calls, 2, 'the continuation starts before changing the provider');
  history.provider = 'codex'; await history.load('A');
  page.resolve({ entries: [{ session_id: 'stale', provider: 'claude' }], next_cursor: null }); await loading;
  assert.equal(history.entries.length, 1);
  assert.equal(history.entries[0].provider, 'codex');
});
