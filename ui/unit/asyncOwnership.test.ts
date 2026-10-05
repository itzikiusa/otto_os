import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';
import * as sessionScope from '../src/lib/stores/sessionScope.ts';
import * as sessionPatch from '../src/lib/stores/sessionPatch.ts';
import * as sessionBuckets from '../src/lib/stores/sessionBuckets.ts';

/** `whenIdle` as node has it (no requestIdleCallback): a 200 ms timer. */
const lazyComponent = {
  whenIdle(fn: () => void): () => void {
    const t = setTimeout(fn, 200);
    return () => clearTimeout(t);
  },
};

function workspace(mayChangeWorkspace: () => boolean | Promise<boolean> = () => true) {
  const requests: { path: string; result: ReturnType<typeof deferred<any[]>> }[] = [];
  const restored: string[] = [];
  const deleted: string[] = [];
  const layout = { panes: [], focusedIndex: 0, bindKey() {}, restore: (id: string) => restored.push(id), retain() {} };
  const api = { del: async (path: string) => { deleted.push(path); }, get: (path: string) => {
    // Scratch, the archived probe and fetch-by-id answer at once; the main
    // (shown) list of the selected workspace is the deferred under test.
    if (path.includes('/scratch/') || path.includes('archived=true') || path.startsWith('/sessions?ids=')) return Promise.resolve([]);
    const result = deferred<any[]>(); requests.push({ path, result }); return result.promise;
  } };
  const { ws } = loadSource(new URL('../src/lib/stores/workspace.svelte.ts', import.meta.url), {
    '../api/client': { api }, '../api/workflows': { listActiveWorkflowRuns: async () => [] }, '../api/workspaces': { fetchWorkspace: async () => ({}) },
    '../router.svelte': { router: { mayChangeWorkspace } }, '../toast.svelte': { toasts: {} }, '../toastError': { toastError: () => {} }, '../plural': { plural: (n: number, w: string) => `${n} ${w}` }, '../confirm.svelte': { confirmer: {} },
    './ui.svelte': { ui: { sessionIsolation: false }, clientId: () => 'test' },
    '../win': { winKey: (key: string) => key }, './splitLayout.svelte': { layout }, './splitLayout': { MAX_PANES: 15, LS_PANES: 'otto_panes_' },
    '../storage': { lsGet: () => null, lsSet() {}, lsRemove() {} },
    '../desktop': { isEmbedded: false }, './sessionScope': sessionScope, './sessionPatch': sessionPatch, './sessionBuckets': sessionBuckets, '../lazy-component.svelte': lazyComponent,
  });
  ws.refreshOtherSessions = async () => {};
  return { ws, requests, restored, deleted, api };
}

/** The store with a saved workspace id and a transport that logs every GET in
 *  order; `/workspaces` is held until the test releases it. */
function bootWorkspace(saved: string, list: { id: string }[], archivedIn: string[] = []) {
  const log: string[] = [];
  const held = deferred<any[]>();
  const api = { get: (path: string) => {
    log.push(path);
    if (path === '/workspaces') return held.promise;
    if (path === '/workspaces/scratch') return Promise.resolve({ id: 'scratch' });
    if (path.includes('archived=true&limit=1')) {
      const ws = path.split('/')[2];
      return Promise.resolve(archivedIn.includes(ws) ? [{ id: `${ws}-old`, workspace_id: ws, archived: true }] : []);
    }
    if (/^\/workspaces\/[^/]+\/sessions\?/.test(path) && !path.includes('archived=true')) {
      const ws = path.split('/')[2];
      return Promise.resolve(ws === 'scratch' ? [] : [{ id: `${ws}-session`, workspace_id: ws, kind: 'agent', status: 'running', last_active_at: 't' }]);
    }
    return Promise.resolve([]);
  } };
  const layout = { panes: [], focusedIndex: 0, bindKey() {}, restore() {}, retain() {} };
  const { ws } = loadSource(new URL('../src/lib/stores/workspace.svelte.ts', import.meta.url), {
    '../api/client': { api, getToken: () => 'tok' }, '../api/workflows': { listActiveWorkflowRuns: async () => [] }, '../api/workspaces': { fetchWorkspace: async () => ({}) },
    '../router.svelte': { router: {} }, '../toast.svelte': { toasts: {} }, '../toastError': { toastError: () => {} }, '../plural': { plural: (n: number, w: string) => `${n} ${w}` }, '../confirm.svelte': { confirmer: {} },
    './ui.svelte': { ui: { sessionIsolation: false }, clientId: () => 'test' },
    '../win': { winKey: (key: string) => key }, './splitLayout.svelte': { layout }, './splitLayout': { MAX_PANES: 15, LS_PANES: 'otto_panes_' },
    '../storage': { lsGet: (k: string) => (k === 'otto_workspace' ? saved : null), lsSet() {}, lsRemove() {} },
    '../desktop': { isEmbedded: false }, './sessionScope': sessionScope, './sessionPatch': sessionPatch, './sessionBuckets': sessionBuckets, '../lazy-component.svelte': lazyComponent,
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

const probes = (log: string[]) => log.filter((p) => p.includes('archived=true&limit=1')).length;

/** The probe runs when idle (no requestIdleCallback in node: a 200 ms timer). */
const idle = () => new Promise((resolve) => setTimeout(resolve, 260));

test('the archived probe stops once it found rows, and keeps asking while there are none (perf G7)', async () => {
  const withRows = bootWorkspace('A', [{ id: 'A' }], ['A']);
  const a = withRows.ws.load();
  withRows.release();
  await a;
  assert.equal(probes(withRows.log), 0, 'off the boot path: no probe before the list has painted');
  await idle();
  const first = probes(withRows.log);
  assert.ok(first >= 1, 'the first list load probes');
  assert.equal(withRows.ws.hasArchived, true);
  await withRows.ws.refreshSessions();
  await withRows.ws.refreshSessions();
  await idle();
  assert.equal(probes(withRows.log), first, 'known archived rows: no probe per refresh');
  assert.equal(withRows.ws.hasArchived, true, 'the folded header stays');

  const none = bootWorkspace('A', [{ id: 'A' }]);
  const b = none.ws.load();
  none.release();
  await b;
  await idle();
  const before = probes(none.log);
  await none.ws.refreshSessions();
  await idle();
  assert.ok(probes(none.log) > before, 'no archived rows yet: a refresh asks again (another client may archive)');
  assert.equal(none.ws.hasArchived, false);
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

test('failed workspace selection clears foreign sessions and retry restores the selected layout', async () => {
  const {ws, requests, restored} = workspace();
  const a = ws.select('A'); requests.find(r => r.path.includes('/A/'))!.result.resolve([{id: 'a', workspace_id: 'A', status: 'idle'}]); await a;
  const b = ws.select('B'); requests.find(r => r.path.includes('/B/'))!.result.reject(new Error('offline')); await b.catch(() => {});
  assert.equal(ws.currentId, 'B');
  assert.equal(ws.sessions.length, 0, 'A must not appear under B');
  assert.match(ws.sessionsError, /offline/);
  const retry = ws.retrySessions();
  requests.filter(r => r.path.includes('/B/')).at(-1)!.result.resolve([{id: 'b', workspace_id: 'B', status: 'idle'}]); await retry;
  assert.equal(ws.sessionsError, null);
  assert.equal(ws.sessions[0].id, 'b');
  assert.equal(restored.at(-1), 'B');
});

test('archive events synchronize membership in two independent windows', () => {
  const a = workspace().ws, b = workspace().ws;
  const session = {id: 's', workspace_id: 'A', kind: 'agent', status: 'idle', meta: {}, archived: false};
  for (const ws of [a, b]) {
    ws.currentId = 'A'; ws.sessions = [session]; ws.archivedLoaded = true;
    ws.closeTab = () => {}; ws.clearNeedsYou = () => {};
    ws.applyEvent({type: 'session_archive_changed', session: {...session, archived: true, status: 'exited'}});
    assert.equal(ws.sessions.length, 0); assert.equal(ws.archivedSessions[0].id, 's');
    ws.applyEvent({type: 'session_archive_changed', session: {...session, archived: false, status: 'reconnectable'}});
    assert.equal(ws.archivedSessions.length, 0); assert.equal(ws.sessions[0].status, 'reconnectable');
  }
});

test('workspace changes ask before identity, tabs or persistence are replaced', async () => {
  const answer=deferred<boolean>();
  const {ws}=workspace(()=>answer.promise);
  ws.currentId='A';ws.sessions=[{id:'kept'}];ws.openTabs=['kept'];ws.layoutReady=true;
  const changing=ws.select('B');
  assert.equal(ws.currentId,'A');assert.deepEqual([...ws.openTabs],['kept']);
  answer.resolve(false);await changing;
  assert.equal(ws.currentId,'A');assert.deepEqual([...ws.openTabs],['kept']);
});

test('same-workspace selection cancels a pending workspace leave decision', async () => {
  const answer=deferred<boolean>();
  const {ws}=workspace(()=>answer.promise);
  ws.currentId='A';ws.sessions=[{id:'kept'}];ws.layoutReady=true;
  const changing=ws.select('B');
  assert.equal(ws.currentId,'A');
  await ws.select('A');answer.resolve(true);await changing;
  assert.equal(ws.currentId,'A');
});

test('a successful workspace leave saves against the original workspace before switching', async () => {
  const answer=deferred<boolean>();
  const {ws}=workspace(()=>answer.promise);
  ws.currentId='A';ws.sessions=[{id:'kept'}];ws.layoutReady=true;
  ws.refreshSessions=async()=>{};ws.waitForSessions=async()=>{};ws.restoreLayout=()=>{};
  ws.refreshActiveWorkflowRuns=async()=>{};
  const changing=ws.select('B');
  assert.equal(ws.currentId,'A','Save still uses A API base');
  answer.resolve(true);await changing;assert.equal(ws.currentId,'B');
});

test('canceling current workspace archive sends no archive request', async () => {
  const answer=deferred<boolean>();
  const {ws,deleted}=workspace(()=>answer.promise);
  ws.currentId='A';ws.workspaces=[{id:'A'},{id:'B'}];
  const removing=ws.archiveWorkspace('A');
  assert.deepEqual(deleted,[]);
  answer.resolve(false);await removing;
  assert.deepEqual(deleted,[]);assert.equal(ws.currentId,'A');assert.equal(ws.workspaces.length,2);
});

test('approved workspace archive asks once and selects its fallback', async () => {
  let decisions=0;
  const {ws,deleted}=workspace(()=>{decisions++;return true;});
  ws.currentId='A';ws.workspaces=[{id:'A'},{id:'B'}];
  ws.refreshSessions=async()=>{};ws.waitForSessions=async()=>{};ws.restoreLayout=()=>{};
  ws.refreshActiveWorkflowRuns=async()=>{};
  await ws.archiveWorkspace('A');
  assert.equal(decisions,1);assert.deepEqual(deleted,['/workspaces/A']);assert.equal(ws.currentId,'B');
});

test('an archive response cannot redirect a newer completed workspace selection', async () => {
  const removed=deferred<void>();
  const {ws,api}=workspace();
  api.del=()=>removed.promise;
  ws.currentId='A';ws.workspaces=[{id:'A'},{id:'B'},{id:'C'}];
  ws.refreshSessions=async()=>{};ws.waitForSessions=async()=>{};ws.restoreLayout=()=>{};
  ws.refreshActiveWorkflowRuns=async()=>{};
  const removing=ws.archiveWorkspace('A');await ws.select('C');
  removed.resolve();await removing;assert.equal(ws.currentId,'C');
});

test('archiving the last workspace asks once then enters scratch context', async () => {
  let decisions=0;
  const {ws}=workspace(()=>{decisions++;return true;});
  ws.currentId='A';ws.workspaces=[{id:'A'}];
  ws.refreshSessions=async()=>{};ws.waitForSessions=async()=>{};ws.restoreLayout=()=>{};
  await ws.archiveWorkspace('A');assert.equal(decisions,1);assert.equal(ws.currentId,null);
});
