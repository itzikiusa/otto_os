// brokers.load is workspace-scoped and latest-wins: A's slower cluster list
// must never land under B (Test / Remove / Move act on those ids by id alone),
// a switch drops A's clusters, tabs and selection, and a failed B load can't
// leave A's list on B's page.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';
import { loadErrorText } from '../src/lib/loadError.ts';

function setup() {
  const pending = new Map<string, ReturnType<typeof deferred<any>>[]>();
  const ws = { currentId: 'A' as string | null };
  const api = {
    get: (url: string) => {
      const d = deferred<any>();
      pending.set(url, [...(pending.get(url) ?? []), d]);
      return d.promise;
    },
  };
  const { brokers } = loadSource(new URL('../src/lib/stores/brokers.svelte.ts', import.meta.url), {
    '../api/client': { api },
    './workspace.svelte': { ws },
    '../loadError': { loadErrorText },
  });
  const settle = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };
  const answer = async (url: string, value: unknown) => {
    const d = (pending.get(url) ?? []).shift();
    assert.ok(d, `no pending GET ${url}`);
    d.resolve(value);
    await settle();
  };
  const fail = async (url: string, e: unknown) => {
    const d = (pending.get(url) ?? []).shift();
    assert.ok(d, `no pending GET ${url}`);
    d.reject(e);
    await settle();
  };
  return { brokers, ws, answer, fail };
}

const clusters = (ws: string) => `/workspaces/${ws}/brokers/clusters`;
const sections = (ws: string) => `/workspaces/${ws}/connection-sections`;

test('a late cluster list from the previous workspace never lands after a switch', async () => {
  const h = setup();
  const a = h.brokers.load('A');
  h.ws.currentId = 'B';
  const b = h.brokers.load('B');
  await h.answer(clusters('B'), [{ id: 'b1' }]);
  await h.answer(sections('B'), [{ id: 'sb' }]);
  await b;
  await h.answer(clusters('A'), [{ id: 'a1' }]);
  await a;
  assert.deepEqual(h.brokers.clusters.map((c: any) => c.id), ['b1']);
  assert.deepEqual(h.brokers.sections.map((s: any) => s.id), ['sb']);
  assert.equal(h.brokers.loading, false);
});

test('switching workspace drops the old clusters, tabs and selection; a failed load shows only its own error', async () => {
  const h = setup();
  const a = h.brokers.load('A');
  await h.answer(clusters('A'), [{ id: 'a1' }, { id: 'a2' }]);
  await h.answer(sections('A'), [{ id: 'sa' }]);
  await a;
  h.brokers.openIds = ['a1', 'a2'];
  h.brokers.selectedId = 'a2';

  h.ws.currentId = 'B';
  const b = h.brokers.load('B');
  assert.equal(h.brokers.clusters.length, 0);
  assert.equal(h.brokers.sections.length, 0);
  assert.equal(h.brokers.openIds.length, 0);
  assert.equal(h.brokers.selectedId, null);
  await h.fail(clusters('B'), new TypeError('Load failed'));
  await h.answer(sections('B'), []);
  await b;
  assert.equal(h.brokers.clusters.length, 0, "A's clusters are not left listed under B");
  assert.equal(h.brokers.loadError, 'Otto can’t reach the daemon.');
  assert.equal(h.brokers.loading, false);
});

test('a superseded load in the same workspace keeps the newer spinner and list', async () => {
  const h = setup();
  const first = h.brokers.load('A');
  const second = h.brokers.load('A');
  await h.answer(clusters('A'), [{ id: 'old' }]);
  await first;
  assert.equal(h.brokers.clusters.length, 0, 'the older response is dropped');
  assert.equal(h.brokers.loading, true, 'the newer load is still in flight');
  await h.answer(clusters('A'), [{ id: 'new' }]);
  await h.answer(sections('A'), []);
  await second;
  assert.deepEqual(h.brokers.clusters.map((c: any) => c.id), ['new']);
  assert.equal(h.brokers.loading, false);
});
