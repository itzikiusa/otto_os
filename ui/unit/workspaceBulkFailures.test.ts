// error-handling-4-08: bulk archive / delete named neither the sessions that
// failed nor why ("2 sessions couldn’t be archived.") — a 403, a 409 and an
// unreachable daemon all read the same.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import * as sessionScope from '../src/lib/stores/sessionScope.ts';
import * as sessionPatch from '../src/lib/stores/sessionPatch.ts';
import * as sessionBuckets from '../src/lib/stores/sessionBuckets.ts';
import { loadErrorText } from '../src/lib/loadError.ts';

function load(fail: Record<string, unknown>) {
  const errors: [string, string][] = [];
  const api = {
    post: async (path: string) => {
      const id = path.split('/')[2];
      if (fail[id]) throw fail[id];
      return { id, workspace_id: 'w', kind: 'agent', status: 'exited', title: id, archived: true };
    },
    del: async (path: string) => {
      const id = path.split('/')[2];
      if (fail[id]) throw fail[id];
    },
    get: async () => [],
    patch: async () => ({}),
  };
  const toasts = { info() {}, success() {}, push() { return 1; }, error(t: string, b: string) { errors.push([t, b]); } };
  const layout = { panes: [], focusedIndex: 0, bindKey() {}, restore() {}, retain() {}, replaceSession() {}, persist() {}, removeSession() {}, closeSession() {} };
  const mod = loadSource(new URL('../src/lib/stores/workspace.svelte.ts', import.meta.url), {
    '../api/client': { api, getToken: () => 'tok' }, './workflowRuns.svelte': { workflowRuns: { list: [], refresh: async () => {}, apply() {}, clear() {} } }, '../api/workspaces': { fetchWorkspace: async () => ({}) },
    '../router.svelte': { router: { module: 'agents', parts: ['agents'], go() {} } }, '../toast.svelte': { toasts }, '../toastError': { toastError: () => {} },
    '../plural': { plural: (n: number, w: string) => `${n} ${w}${n === 1 ? '' : 's'}` }, '../confirm.svelte': { confirmer: { ask: async () => true } },
    './ui.svelte': { ui: { sessionIsolation: false, closeTabPref: '', setCloseTabPref() {} }, clientId: () => 'test' },
    '../win': { winKey: (key: string) => key }, './splitLayout.svelte': { layout }, './splitLayout': { MAX_PANES: 15, LS_PANES: 'otto_panes_' },
    '../storage': { lsGet: () => null, lsSet() {}, lsRemove() {} }, '../loadError': { loadErrorText },
    '../desktop': { isEmbedded: false }, './sessionScope': sessionScope, './sessionPatch': sessionPatch, './sessionBuckets': sessionBuckets,
    '../lazy-component.svelte': { whenIdle: () => () => {} },
  });
  const { ws } = mod;
  ws.sessions = ['a', 'b', 'c'].map((id) => ({ id, workspace_id: 'w', kind: 'agent', status: 'idle', title: `Agent ${id}`, archived: false }));
  return { ws, errors, bulkFailureText: mod.bulkFailureText as (f: { title: string; cause: string }[]) => string };
}

const forbidden = Object.assign(new Error('forbidden'), { status: 403 });
const conflict = Object.assign(new Error('The session is busy.'), { status: 409 });

test('bulk archive names the failed session and its cause', async () => {
  const { ws, errors } = load({ b: forbidden });
  assert.equal(await ws.archiveSessions(['a', 'b']), 1);
  assert.deepEqual(errors, [['Couldn’t archive', 'Agent b: You don’t have access to this.']]);
});

test('bulk delete says a shared cause once, and different causes as first + "and N more"', async () => {
  const down = new TypeError('Failed to fetch');
  const same = load({ a: down, b: down });
  assert.equal(await same.ws.killSessions(['a', 'b']), 2);
  assert.deepEqual(same.errors, [['Couldn’t delete', '2 sessions: Otto can’t reach the daemon.']]);
  const mixed = load({ a: conflict, c: forbidden });
  assert.equal(await mixed.ws.killSessions(['a', 'b', 'c']), 2);
  assert.deepEqual(mixed.errors, [['Couldn’t delete', 'Agent a: The session is busy — and 1 more.']]);
  assert.equal(mixed.bulkFailureText([]), '');
});
