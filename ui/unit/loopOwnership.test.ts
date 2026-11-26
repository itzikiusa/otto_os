import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

function fixture() {
  const calls: { method: string; path: string; result: ReturnType<typeof deferred<any>> }[] = [];
  const request = (method: string) => (path: string) => {
    const result = deferred<any>(); calls.push({ method, path, result }); return result.promise;
  };
  const get = request('GET');
  const { loops } = loadSource(new URL('../src/lib/stores/loops.svelte.ts', import.meta.url), {
    '../api/client': { api: { get, bg: { get }, post: request('POST'), patch: request('PATCH') } },
    '../loadError': { loadErrorText: String }, '../live': { liveQuery() {} },
    '../lazyModule': { announceModule() {} },
  });
  return { loops, calls };
}
const settle = () => new Promise<void>((resolve) => setImmediate(resolve));
const detail = (id: string) => ({ loop: { id, workspace_id: 'A' }, iterations: [] });

test('closing detail discards queued reruns and late iteration bodies', async () => {
  const { loops, calls } = fixture();
  const first = loops.loadDetail('a'); const second = loops.loadDetail('a');
  const iteration = loops.loadIteration('a', { idx: 1, status: 'done' });
  loops.closeDetail();
  calls[0].result.resolve(detail('a')); calls[1].result.resolve({ idx: 1, plan: 'old plan' });
  await first; await iteration; await settle();
  const total = calls.length;
  // Settle an erroneous rerun so failure never leaves the harness pending.
  for (const call of calls.slice(2)) call.result.resolve(detail('a'));
  await second;
  assert.equal(total, 2, 'close must not launch a queued request');
  assert.equal(loops.detail, null);
  assert.equal(Object.keys(loops.fullIterations).length, 0);
});

test('opening B invalidates A immediately while B waits for the single-flight request', async () => {
  const { loops, calls } = fixture();
  const a = loops.loadDetail('a'); const b = loops.loadDetail('b');
  calls[0].result.resolve(detail('a')); await a; await settle();
  assert.equal(loops.detail, null, 'an A response must not publish after B was requested');
  calls[1].result.resolve(detail('b')); await b;
  assert.equal(loops.detail.loop.id, 'b');
});

