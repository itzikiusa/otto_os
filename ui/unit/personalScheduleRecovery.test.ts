import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

function setup(schedules: (id: string) => Promise<unknown[]>) {
  return loadSource(new URL('../src/lib/stores/personalAgents.svelte.ts', import.meta.url), {
    '../api/personalAgents': { personalAgentsApi: { schedules } },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../lazyModule': { announceModule() {} },
  }).personalAgents;
}

test('failed schedule refresh preserves known cadence and exposes a recoverable error', async () => {
  let fails = false;
  const store = setup(async () => { if (fails) throw new Error('offline'); return [{ id: 'daily' }]; });
  await store.loadSchedules('a');
  fails = true;
  await store.loadSchedules('a');
  assert.equal(store.schedulesByAgent.a[0]?.id, 'daily');
  assert.match(store.schedulesError.a, /offline/);
  fails = false;
  await store.loadSchedules('a');
  assert.equal(store.schedulesError.a, undefined);
  assert.equal(store.schedulesLoading.a, false);
});

test('stale schedule failure cannot finish or replace the newer loading state', async () => {
  const old = deferred<unknown[]>(), fresh = deferred<unknown[]>(); let n = 0;
  const store = setup(() => ++n === 1 ? old.promise : fresh.promise);
  const first = store.loadSchedules('a'), second = store.loadSchedules('a');
  old.reject(new Error('obsolete')); await first;
  assert.equal(store.schedulesLoading.a, true);
  assert.equal(store.schedulesError.a, undefined);
  fresh.resolve([]); await second;
  assert.equal(store.schedulesLoading.a, false);
});

test('agent list detail fanout is bounded and obsolete queued work is skipped', async () => {
  const release = deferred<unknown[]>();
  let active = 0, peak = 0, calls = 0;
  const store = loadSource(new URL('../src/lib/stores/personalAgents.svelte.ts', import.meta.url), {
    '../api/personalAgents': { personalAgentsApi: {
      list: async (ws: string) => ws === 'old' ? Array.from({ length: 1000 }, (_, i) => ({ id: `a${i}` })) : [],
      schedules: async () => { calls++; peak = Math.max(peak, ++active); await release.promise; active--; return []; },
    } },
    '../loadError': { loadErrorText: String }, '../lazyModule': { announceModule() {} },
  }).personalAgents;
  const old = store.loadAgents('old');
  await new Promise((resolve) => setImmediate(resolve));
  await store.loadAgents('new');
  release.resolve([]); await old;
  assert.ok(peak <= 2, `expected at most two schedule requests, got ${peak}`);
  assert.equal(calls, 2, 'obsolete queued agents must not consume network requests');
  assert.equal(store.agents.length, 0);
});
