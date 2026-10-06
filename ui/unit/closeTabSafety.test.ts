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
  // A tiny fake daemon: archive/unarchive flip the row, GET reads it back.
  const row = { id: 's1', workspace_id: 'w', kind: 'agent', status: 'idle', title: 'Agent one', archived: false };
  const api = {
    del: async (path: string) => { deleted.push(path); },
    post: async (path: string) => {
      posted.push(path);
      if (path.endsWith('/archive')) row.archived = true;
      if (path.endsWith('/unarchive')) row.archived = false;
      return { ...row };
    },
    patch: async (path: string) => { posted.push(path); return {}; },
    get: async (path: string) => (path === '/sessions/s1' ? { ...row } : path.startsWith('/sessions?ids=') ? [{ ...row }] : []),
  };
  const confirmer = {
    ask: async (msg: string) => { asked.push(msg); return confirmAnswer; },
    choose: async (msg: string) => { asked.push(msg); return { value: null, remember: false }; },
  };
  // Toasts mirror the real store's contract: the timer can be held (hover /
  // focus), and onClose reports 'expired' when it runs out, 'action' on Undo.
  const undo: (() => Promise<void>)[] = [];
  const held: { pause: () => void; resume: () => void }[] = [];
  const toasts = {
    info() {}, success() {}, error() {},
    push(_l: string, _t: string, _b: string, ms: number, o: any) {
      let closed = false;
      let handle: ReturnType<typeof setTimeout> | null = null;
      const close = (reason: string) => { if (closed) return; closed = true; if (handle) clearTimeout(handle); o?.onClose?.(reason); };
      const arm = () => { handle = setTimeout(() => close('expired'), ms); };
      arm();
      held.push({ pause: () => { if (handle) clearTimeout(handle); handle = null; }, resume: arm });
      if (o?.action) undo.push(async () => { close('action'); await o.action.run(); });
      return 1;
    },
  };
  const layout = { panes: [], focusedIndex: 0, bindKey() {}, restore() {}, retain() {}, replaceSession() {}, persist() {}, removeSession() {}, closeSession() {} };
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
  return { ws, deleted, posted, asked, undo, held, row };
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
  ws.cancelPendingDelete('s1');
});

// S13-301: every way back from the archive cancels the pending delete.
test('"Always delete" then ⌘⇧T: the reopened session is never deleted', async () => {
  const { ws, deleted, posted } = store('agents', 'delete');
  ws.closeTab = (id: string) => { ws.openTabs = ws.openTabs.filter((t: string) => t !== id); ws.recentlyClosed = [...ws.recentlyClosed, id]; };
  await ws.deleteWithUndo('s1', 20);
  assert.ok(ws.recentlyClosed.includes('s1'));
  await ws.reopenClosedTab();
  assert.ok(posted.includes('/sessions/s1/unarchive'), 'reopen unarchived it');
  await new Promise((r) => setTimeout(r, 60));
  assert.deepEqual(deleted, [], 'no DELETE after the grace period');
});

test('"Always delete" then Archived ▸ Restore: no DELETE', async () => {
  const { ws, deleted } = store('agents', 'delete');
  await ws.deleteWithUndo('s1', 20);
  await ws.unarchiveSession('s1');
  await new Promise((r) => setTimeout(r, 60));
  assert.deepEqual(deleted, []);
});

test('"Always delete" then another window restores it: the re-check keeps it', async () => {
  const { ws, deleted, row } = store('agents', 'delete');
  await ws.deleteWithUndo('s1', 20);
  row.archived = false; // restored elsewhere; this window missed the event
  await new Promise((r) => setTimeout(r, 60));
  assert.deepEqual(deleted, [], 'the server row is live again: never DELETE');
});

test('a toast held open past the grace period still lets Undo win', async () => {
  const { ws, deleted, posted, undo, held } = store('agents', 'delete');
  ws.unarchiveAndOpen = async () => { posted.push('unarchive'); };
  await ws.deleteWithUndo('s1', 20);
  held[0].pause(); // pointer on the toast
  await new Promise((r) => setTimeout(r, 60));
  assert.deepEqual(deleted, [], 'a paused toast pauses the delete');
  await undo[0]();
  assert.deepEqual(posted, ['/sessions/s1/archive', 'unarchive']);
  await new Promise((r) => setTimeout(r, 40));
  assert.deepEqual(deleted, []);
});

test('sessionVerbsApply is Agents-only', () => {
  assert.equal(sessionScope.sessionVerbsApply('agents'), true);
  assert.equal(sessionScope.sessionVerbsApply('git'), false);
});

// S14-301: Classrooms' back row and other workspaces draw "working" from their
// own list; the store's statusMap never saw those rows.
test('requestArchive asks for a working row the store has no status for', async () => {
  const { ws, posted, asked } = store('home', '', false);
  ws.sessions = [];
  ws.statusMap = {};
  assert.equal(await ws.requestArchive('bg1', { working: true, title: 'Step 3', engine: 'workflow' }), false);
  assert.equal(asked.length, 1, 'the working guard asked');
  assert.match(asked[0], /“Step 3” is working right now/);
  assert.match(asked[0], /workflow run that owns it/);
  assert.deepEqual(posted, [], 'cancel archives nothing');
});

test('requestArchive: an idle unknown row archives at once; a busy shell never asks', async () => {
  const idle = store('home', '');
  idle.ws.sessions = [];
  idle.ws.statusMap = {};
  assert.equal(await idle.ws.requestArchive('s9', { working: false, title: 'x', engine: null }), true);
  assert.deepEqual(idle.asked, []);
  assert.deepEqual(idle.posted, ['/sessions/s9/archive']);
  const shell = store('agents', '');
  shell.ws.sessions = [{ id: 's1', workspace_id: 'w', kind: 'agent', provider: 'shell', status: 'working', title: 'zsh', archived: false }];
  shell.ws.statusMap = { s1: 'working' };
  assert.equal(await shell.ws.requestArchive('s1'), true);
  assert.equal(await shell.ws.confirmRestart('s1'), true, 'restart uses the same guard as archive');
  assert.deepEqual(shell.asked, []);
});

test('requestArchive and confirmRestart ask for a working agent', async () => {
  const { ws, asked } = store('agents', '', false);
  ws.sessions = [{ id: 's1', workspace_id: 'w', kind: 'agent', provider: 'claude', status: 'working', title: 'Agent one', archived: false }];
  ws.statusMap = { s1: 'working' };
  assert.equal(await ws.requestArchive('s1'), false);
  assert.equal(await ws.confirmRestart('s1'), false);
  assert.equal(asked.length, 2);
});
