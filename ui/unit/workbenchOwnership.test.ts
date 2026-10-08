import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

function fixture() {
  const calls: { method: string; ws: string; id?: string; body?: any; result: ReturnType<typeof deferred<any>> }[] = [];
  const request = (method: string) => (ws: string, id?: any, body?: any) => {
    const result = deferred<any>(); calls.push({ method, ws, id, body, result }); return result.promise;
  };
  const { workbench: store } = loadSource(new URL('../src/modules/workbench/workbench.svelte.ts', import.meta.url), {
    svelte: { untrack: (fn: () => unknown) => fn() },
    '../../lib/api/workbench': Object.fromEntries(['create', 'get', 'list', 'purge', 'restore', 'trash', 'update'].map((m) => [`${m}Workbench${m === 'list' ? 'Docs' : 'Doc'}`, request(m)])),
    '../../lib/loadError': { loadErrorText: String }, '../../lib/api/client': { ApiError: class extends Error {} },
    '../../lib/live': { onLive: () => () => {}, appLive: { onResync: () => () => {} } },
  });
  const doc = (id = 'a', content = 'base') => ({ id, name: id, content, content_hash: content, workspace_id: 'A', updated_at: '', rev: 1 });
  const open = () => {
    store.ws = 'A'; store.docs = [doc()]; store.tabs = ['a']; store.active = 'a';
    store.open.a = { id: 'a', doc: doc(), buffer: 'base', saved: 'base', loading: false, saving: false, remoteChanged: false };
  };
  return { store, calls, doc, open };
}

test('create finishing after workspace switch cannot add an old doc to the new workspace', async () => {
  const { store, calls, doc, open } = fixture(); open();
  const creating = store.create({ name: 'created' });
  store.ws = 'B'; store.docs = []; store.tabs = []; store.open = {}; store.active = null;
  calls[0].result.resolve(doc('created')); await creating;
  assert.equal(store.docs.length, 0); assert.equal(store.tabs.length, 0); assert.equal(store.active, null);
});

test('metadata and restore completions remain with their originating workspace', async () => {
  for (const operation of ['patchMeta', 'restoreFromTrash']) {
    const { store, calls, doc, open } = fixture(); open();
    const changing = store[operation]('a', { name: 'renamed' });
    store.ws = 'B'; store.docs = []; store.open = {};
    calls[0].result.resolve(doc()); await changing;
    assert.equal(store.docs.length, 0, operation);
  }
});

test('remote reload cannot overwrite edits made while its response is pending', async () => {
  const { store, calls, doc, open } = fixture(); open();
  const loading = store.reload('a');
  store.setBuffer('a', 'typed during reload');
  calls[0].result.resolve(doc('a', 'remote')); await loading;
  assert.equal(store.open.a.buffer, 'typed during reload');
  assert.equal(store.open.a.remoteChanged, true);
  assert.equal(store.open.a.loading, false);
});

test('failed initial list preserves persisted tabs for Retry', async () => {
  const { store, calls } = fixture();
  const attaching = store.attach('A'); store.tabs = ['saved-tab']; store.active = 'saved-tab';
  calls[0].result.reject(new Error('offline')); await attaching;
  assert.deepEqual(Array.from(store.tabs), ['saved-tab']); assert.equal(store.active, 'saved-tab');
});

test('trash does not discard a dirty buffer after its save failed', async () => {
  const { store, calls, open } = fixture(); open(); store.open.a.buffer = 'unsaved';
  const trashing = store.moveToTrash('a').catch(() => {});
  calls[0].result.reject(new Error('offline'));
  await new Promise<void>((r) => setImmediate(r));
  for (const call of calls.slice(1)) call.result.resolve({ id: 'a' });
  await trashing;
  assert.equal(calls.some((c) => c.method === 'trash'), false);
  assert.equal(store.open.a.buffer, 'unsaved');
});

test('ordinary reload and same-workspace create still publish', async () => {
  const { store, calls, doc, open } = fixture(); open();
  const loading = store.reload('a'); calls[0].result.resolve(doc('a', 'remote')); await loading;
  assert.equal(store.open.a.buffer, 'remote');
  const creating = store.create({}); calls[1].result.resolve(doc('new')); await creating;
  assert.equal(store.active, 'new'); assert.equal(store.docs[0].id, 'new');
});
