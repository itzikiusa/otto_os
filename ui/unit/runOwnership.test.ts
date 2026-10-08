import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

function fixture() {
  const calls: { method: string; id: string; result: ReturnType<typeof deferred<any>> }[] = [];
  const request = (method: string) => (id: string) => {
    const result = deferred<any>(); calls.push({ method, id, result }); return result.promise;
  };
  const { runWithOtto: store } = loadSource(new URL('../src/lib/stores/runWithOtto.svelte.ts', import.meta.url), {
    '../api/runWithOtto': { runWithOttoApi: Object.fromEntries(['list', 'get', 'events', 'launch', 'cancel', 'approve'].map((name) => [name, request(name)])) },
    '../loadError': { loadErrorText: String }, '../lazyModule': { announceModule() {} },
  });
  return { store, calls };
}
const settle = () => new Promise<void>((resolve) => setImmediate(resolve));

test('old list response cannot replace a newer request in the same workspace', async () => {
  const { store, calls } = fixture();
  const first = store.loadList('A'), second = store.loadList('A');
  calls[1].result.resolve([{ id: 'a', status: 'canceled' }]); await second;
  calls[0].result.resolve([{ id: 'a', status: 'executing' }]); await first;
  assert.equal(store.list[0].status, 'canceled');
});

test('launch completion does not switch the store back to a departed workspace', async () => {
  const { store, calls } = fixture();
  const a = store.loadList('A'); calls[0].result.resolve([]); await a;
  const launch = store.launch('A', {});
  const b = store.loadList('B'); calls[2].result.resolve([{ id: 'b', workspace_id: 'B' }]); await b;
  calls[1].result.resolve({ id: 'a', workspace_id: 'A' }); await settle();
  const count = calls.length;
  for (const call of calls.slice(3)) call.result.resolve([{ id: 'a', workspace_id: 'A' }]);
  await launch;
  assert.equal(count, 3, 'old launch must not fetch another workspace');
  assert.equal(store.list[0].id, 'b');
  assert.equal(store.byId.a, undefined);
});

test('late refresh cannot undo a confirmed cancellation', async () => {
  const { store, calls } = fixture();
  const list = store.loadList('A'); calls[0].result.resolve([{ id: 'a', workspace_id: 'A', status: 'executing' }]); await list;
  const stale = store.refreshRun('a'), canceled = store.cancel('a');
  calls[2].result.resolve({ id: 'a', workspace_id: 'A', status: 'canceled' }); await canceled;
  calls[1].result.resolve({ id: 'a', workspace_id: 'A', status: 'executing' }); await stale;
  for (const call of calls.slice(3)) call.result.resolve([]);
  assert.equal(store.byId.a.status, 'canceled');
});

test('workspace change clears detail and rejects late event history', async () => {
  const { store, calls } = fixture();
  const list = store.loadList('A'); calls[0].result.resolve([{ id: 'a', workspace_id: 'A' }]); await list;
  const opening = store.open('a');
  const b = store.loadList('B'); calls[3].result.resolve([{ id: 'b', workspace_id: 'B' }]); await b;
  calls[1].result.resolve({ id: 'a', workspace_id: 'A' }); calls[2].result.resolve([{ id: 'old-event' }]); await opening;
  assert.equal(store.openRun, null);
  assert.equal(store.byId.a, undefined);
  assert.equal(store.eventsByRun.a, undefined);
});
