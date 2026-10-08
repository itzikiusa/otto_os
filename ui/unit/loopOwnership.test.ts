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

for (const action of ['pause', 'updateLimits']) {
  test(`delayed ${action} cannot insert an old loop into a new workspace`, async () => {
    const { loops, calls } = fixture();
    const a = loops.loadList('A'); calls[0].result.resolve([{ id: 'a', workspace_id: 'A' }]); await a;
    const pending = action === 'pause' ? loops.pause('a') : loops.updateLimits('a', { max_iterations: 4 });
    const b = loops.loadList('B'); calls[2].result.resolve([{ id: 'b', workspace_id: 'B' }]); await b;
    calls[1].result.resolve({ id: 'a', workspace_id: 'A', status: 'paused' }); await pending;
    assert.deepEqual(Array.from(loops.list, (row: any) => row.id), ['b']);
  });
}

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

for (const action of ['verifyCriterion', 'answerQuestion', 'retryExecutor']) {
  test(`late ${action} does not reopen a closed detail`, async () => {
    const { loops, calls } = fixture();
    const opening = loops.loadDetail('a'); calls[0].result.resolve(detail('a')); await opening;
    const pending = loops[action]('a', 'criterion', 'evidence');
    loops.closeDetail(); calls[1].result.resolve({}); await settle();
    const total = calls.length;
    for (const call of calls.slice(2)) call.result.resolve(detail('a'));
    await pending;
    assert.equal(total, 2); assert.equal(loops.detail, null);
  });
}
