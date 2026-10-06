import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import * as sessionScope from '../src/lib/stores/sessionScope.ts';
import * as sessionPatch from '../src/lib/stores/sessionPatch.ts';
import * as sessionBuckets from '../src/lib/stores/sessionBuckets.ts';

// S13-01: ⌘W / File ▸ Close Tab / End Session used to end the focused pane's
// session from ANY page, and under "Always delete" did so silently.

function store(module: string, pref: '' | 'archive' | 'delete', confirmAnswer = true) {
  const deleted: string[] = [];
  const posted: string[] = [];
  const asked: string[] = [];
  const api = {
    del: async (path: string) => { deleted.push(path); },
    post: async (path: string) => { posted.push(path); return {}; },
    patch: async (path: string) => { posted.push(path); return {}; },
    get: async () => [],
  };
  const confirmer = {
    ask: async (msg: string) => { asked.push(msg); return confirmAnswer; },
    choose: async (msg: string) => { asked.push(msg); return { value: null, remember: false }; },
  };
  const undo: (() => Promise<void>)[] = [];
  const toasts = { info() {}, success() {}, push(_l: string, _t: string, _b: string, _ms: number, o: any) { if (o?.action) undo.push(o.action.run); } };
  const layout = { panes: [], focusedIndex: 0, bindKey() {}, restore() {}, retain() {}, replaceSession() {} };
  const { ws } = loadSource(new URL('../src/lib/stores/workspace.svelte.ts', import.meta.url), {
    '../api/client': { api, getToken: () => 'tok' }, '../api/workflows': { listActiveWorkflowRuns: async () => [] }, '../api/workspaces': { fetchWorkspace: async () => ({}) },
    '../router.svelte': { router: { module, parts: [module], go() {} } }, '../toast.svelte': { toasts }, '../toastError': { toastError: () => {} }, '../plural': { plural: (n: number, w: string) => `${n} ${w}` }, '../confirm.svelte': { confirmer },
    './ui.svelte': { ui: { sessionIsolation: false, closeTabPref: pref, setCloseTabPref() {} }, clientId: () => 'test' },
    '../win': { winKey: (key: string) => key }, './splitLayout.svelte': { layout }, './splitLayout': { MAX_PANES: 15, LS_PANES: 'otto_panes_' },
    '../storage': { lsGet: () => null, lsSet() {}, lsRemove() {} },
    '../desktop': { isEmbedded: false }, './sessionScope': sessionScope, './sessionPatch': sessionPatch, './sessionBuckets': sessionBuckets,
    '../lazy-component.svelte': { whenIdle: () => () => {} },
  });
  ws.sessions = [{ id: 's1', workspace_id: 'w', kind: 'agent', status: 'idle', title: 'Agent one', archived: false }];
  ws.statusMap = { s1: 'idle' };
  ws.activeSessionId = 's1';
  ws.openTabs = ['s1'];
  return { ws, deleted, posted, asked, undo };
}

const tick = () => new Promise((r) => setImmediate(r));

test('⌘W on a non-Agents page never touches the hidden session', async () => {
  for (const module of ['git', 'vault', 'settings', 'home']) {
    const { ws, deleted, posted, asked } = store(module, 'delete');
    assert.equal(ws.closeActiveTab(), false, module);
    await tick();
    assert.deepEqual(deleted, [], `${module}: nothing deleted`);
    assert.deepEqual(posted, [], `${module}: nothing archived`);
    assert.deepEqual(asked, [], `${module}: no dialog about a session the user cannot see`);
  }
});

test('"Always delete": no dialog, but archive first and delete only after the Undo window', async () => {
  const { ws, deleted, posted, asked } = store('agents', 'delete');
  ws.applyArchiveState = () => {};
  ws.unarchiveAndOpen = async () => { posted.push('unarchive'); };
  await ws.deleteWithUndo('s1', 20);
  assert.deepEqual(posted, ['/sessions/s1/archive'], 'archived (stopped, history kept) at once');
  assert.deepEqual(deleted, [], 'not deleted yet');
  await new Promise((r) => setTimeout(r, 40));
  assert.deepEqual(deleted, ['/sessions/s1'], 'deleted when the Undo window ran out');
  assert.deepEqual(asked, [], 'the remembered choice is honoured: no dialog');
});

test('Undo inside the window keeps the session', async () => {
  const { ws, deleted, posted, undo } = store('agents', 'delete');
  ws.applyArchiveState = () => {};
  ws.unarchiveAndOpen = async () => { posted.push('unarchive'); };
  await ws.deleteWithUndo('s1', 20);
  assert.equal(undo.length, 1);
  await undo[0]();
  await new Promise((r) => setTimeout(r, 40));
  assert.deepEqual(deleted, [], 'never deleted');
  assert.deepEqual(posted, ['/sessions/s1/archive', 'unarchive']);
});

test('⌘W under "Always delete" goes through the undoable path', async () => {
  const { ws, deleted, posted, asked } = store('agents', 'delete');
  ws.applyArchiveState = () => {};
  assert.equal(ws.closeActiveTab(), true);
  await tick();
  assert.deepEqual(asked, []);
  assert.deepEqual(posted, ['/sessions/s1/archive']);
  assert.deepEqual(deleted, [], 'a stray ⌘W is never an immediate irreversible delete');
  ws.pendingDeletes.forEach((h: ReturnType<typeof setTimeout>) => clearTimeout(h));
});

test('sessionVerbsApply is Agents-only', () => {
  assert.equal(sessionScope.sessionVerbsApply('agents'), true);
  assert.equal(sessionScope.sessionVerbsApply('git'), false);
});
