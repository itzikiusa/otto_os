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
