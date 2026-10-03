import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';
import * as sessionScope from '../src/lib/stores/sessionScope.ts';
import * as sessionPatch from '../src/lib/stores/sessionPatch.ts';
import * as sessionBuckets from '../src/lib/stores/sessionBuckets.ts';

function workspace() {
  const requests: { path: string; result: ReturnType<typeof deferred<any[]>> }[] = [];
  const restored: string[] = [];
  const layout = { panes: [], focusedIndex: 0, bindKey() {}, restore: (id: string) => restored.push(id), retain() {} };
  const api = { get: (path: string) => {
    // Scratch, the archived probe and fetch-by-id answer at once; the main
    // (shown) list of the selected workspace is the deferred under test.
    if (path.includes('/scratch/') || path.includes('archived=true') || path.startsWith('/sessions?ids=')) return Promise.resolve([]);
    const result = deferred<any[]>(); requests.push({ path, result }); return result.promise;
  } };
  const { ws } = loadSource(new URL('../src/lib/stores/workspace.svelte.ts', import.meta.url), {
    '../api/client': { api }, '../api/workflows': { listActiveWorkflowRuns: async () => [] }, '../api/workspaces': { fetchWorkspace: async () => ({}) },
    '../router.svelte': { router: {} }, '../toast.svelte': { toasts: {} }, '../confirm.svelte': { confirmer: {} },
    './ui.svelte': { ui: { sessionIsolation: false }, clientId: () => 'test' },
    '../win': { winKey: (key: string) => key }, './splitLayout.svelte': { layout }, './splitLayout': { MAX_PANES: 15, LS_PANES: 'otto_panes_' },
    '../storage': { lsGet: () => null, lsSet() {}, lsRemove() {} },
    '../desktop': { isEmbedded: false }, './sessionScope': sessionScope, './sessionPatch': sessionPatch, './sessionBuckets': sessionBuckets,
  });
  ws.refreshOtherSessions = async () => {};
  return { ws, requests, restored };
}

/** The store with a saved workspace id and a transport that logs every GET in
 *  order; `/workspaces` is held until the test releases it. */
function bootWorkspace(saved: string, list: { id: string }[]) {
  const log: string[] = [];
  const held = deferred<any[]>();
  const api = { get: (path: string) => {
    log.push(path);
    if (path === '/workspaces') return held.promise;
    if (path === '/workspaces/scratch') return Promise.resolve({ id: 'scratch' });
    if (/^\/workspaces\/[^/]+\/sessions\?/.test(path) && !path.includes('archived=true')) {
      const ws = path.split('/')[2];
      return Promise.resolve(ws === 'scratch' ? [] : [{ id: `${ws}-session`, workspace_id: ws, kind: 'agent', status: 'running', last_active_at: 't' }]);
    }
    return Promise.resolve([]);
  } };
  const layout = { panes: [], focusedIndex: 0, bindKey() {}, restore() {}, retain() {} };
  const { ws } = loadSource(new URL('../src/lib/stores/workspace.svelte.ts', import.meta.url), {
    '../api/client': { api, getToken: () => 'tok' }, '../api/workflows': { listActiveWorkflowRuns: async () => [] }, '../api/workspaces': { fetchWorkspace: async () => ({}) },
    '../router.svelte': { router: {} }, '../toast.svelte': { toasts: {} }, '../confirm.svelte': { confirmer: {} },
    './ui.svelte': { ui: { sessionIsolation: false }, clientId: () => 'test' },
    '../win': { winKey: (key: string) => key }, './splitLayout.svelte': { layout }, './splitLayout': { MAX_PANES: 15, LS_PANES: 'otto_panes_' },
    '../storage': { lsGet: (k: string) => (k === 'otto_workspace' ? saved : null), lsSet() {}, lsRemove() {} },
    '../desktop': { isEmbedded: false }, './sessionScope': sessionScope, './sessionPatch': sessionPatch, './sessionBuckets': sessionBuckets,
  });
  ws.refreshOtherSessions = async () => {};
  ws.refreshActiveWorkflowRuns = async () => {};
  return { ws, log, release: () => held.resolve(list) };
}

const shown = (log: string[], ws: string) => log.filter((p) => p.startsWith(`/workspaces/${ws}/sessions?`) && !p.includes('archived=true'));

test('boot: the saved workspace\'s session list goes out with /workspaces, and is used once (perf G3)', async () => {
  const { ws, log, release } = bootWorkspace('A', [{ id: 'A' }, { id: 'B' }]);
  const loading = ws.load();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(shown(log, 'A').length, 1, `sessions start before /workspaces answers: ${log.join(' ')}`);
  assert.equal(shown(log, 'scratch').length, 1, 'with the scratch list');
  release();
  await loading;
  assert.equal(ws.currentId, 'A');
  assert.equal(ws.sessions[0]?.id, 'A-session');
  assert.equal(shown(log, 'A').length, 1, 'select() reused the boot request instead of asking again');
  assert.equal(shown(log, 'scratch').length, 1);
  await ws.refreshSessions();
  assert.equal(shown(log, 'A').length, 2, 'one-shot: a later refresh asks the daemon');
});

test('boot: a saved workspace that no longer exists drops its speculative list (perf G3)', async () => {
  const { ws, log, release } = bootWorkspace('gone', [{ id: 'B' }]);
  const loading = ws.load();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(shown(log, 'gone').length, 1);
  release();
  await loading;
  assert.equal(ws.currentId, 'B');
  assert.equal(ws.sessions[0]?.id, 'B-session', 'the fallback workspace\'s own list, not the stale one');
  assert.equal(shown(log, 'B').length, 1);
  assert.equal(shown(log, 'scratch').length, 2, 'the scratch list is fetched again with the real selection');
});

test('late workspace selection cannot publish sessions or restore the old layout', async () => {
  const { ws, requests, restored } = workspace();
  const a = ws.select('A'); const b = ws.select('B');
  requests[1].result.resolve([{ id: 'B-session' }]); await b;
  requests[0].result.resolve([{ id: 'A-session' }]); await a;
  assert.equal(ws.currentId, 'B');
  assert.equal(ws.sessions[0].id, 'B-session');
  assert.deepEqual(restored, ['B']);
});

test('concurrent refreshes in one workspace coalesce into one trailing load', async () => {
  const { ws, requests } = workspace();
  ws.currentId = 'A';
  const first = ws.refreshSessions(); const second = ws.refreshSessions(); const third = ws.refreshSessions();
  assert.equal(requests.length, 1, 'one request in flight, the rest queue behind it');
  assert.equal(second, third, 'every caller during the flight shares the ONE trailing load');
  assert.match(requests[0].path, /foreground=true/, 'the main list asks for shown sessions only');
  requests[0].result.resolve([{ id: 'old' }]); await first;
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(requests.length, 2, 'the trailing load starts after the first settles');
  requests[1].result.resolve([{ id: 'new' }]); await second;
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
