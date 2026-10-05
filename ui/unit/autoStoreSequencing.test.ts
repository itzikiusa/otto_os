// S17-12: the automation stores' stale-response guards. Each test runs the
// PRODUCTION store with a deferred transport and lands responses out of order.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

const latest = () => loadSource(new URL('../src/lib/latest.ts', import.meta.url), {});

function transport() {
  const calls: { path: string; d: ReturnType<typeof deferred<any>> }[] = [];
  const req = (path: string) => {
    const d = deferred<any>();
    calls.push({ path, d });
    return d.promise;
  };
  const api: any = { get: req, post: req, patch: req, del: req, put: req, bg: { get: req } };
  return { api, calls };
}

test('latestOnly: a superseded ticket is not current and its signal aborts', () => {
  const { latestOnly } = latest();
  const l = latestOnly();
  const a = l.begin();
  const b = l.begin();
  assert.equal(a.current, false);
  assert.equal(a.signal.aborted, true);
  assert.equal(b.current, true);
  l.cancel();
  assert.equal(b.current, false);
});

test('loops: a background poll landing after Pause cannot restore "running"', async () => {
  const { api, calls } = transport();
  const { loops } = loadSource(new URL('../src/lib/stores/loops.svelte.ts', import.meta.url), {
    '../api/client': { api },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../live': { liveQuery: () => ({ stop() {}, now() {} }) },
    '../lazyModule': { announceModule() {} },
    '../latest': latest(),
  });
  const detail = (status: string) => ({ loop: { id: 'L1', status }, iterations: [] });
  // A poll (bg lane) is in flight…
  const poll = (loops as any).fetchDetail('L1', true);
  // …then the user's action re-fetches the detail.
  const action = loops.loadDetail('L1');
  const [bg, fg] = calls;
  fg.d.resolve(detail('paused'));
  await action;
  bg.d.resolve(detail('running'));
  await poll;
  assert.equal(loops.detail.loop.status, 'paused');
  assert.equal(loops.loadingDetail, false);
});

test('loops: answerQuestion encodes the question id', async () => {
  const { api, calls } = transport();
  const { loops } = loadSource(new URL('../src/lib/stores/loops.svelte.ts', import.meta.url), {
    '../api/client': { api },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../live': { liveQuery: () => ({ stop() {}, now() {} }) },
    '../lazyModule': { announceModule() {} },
    '../latest': latest(),
  });
  void loops.answerQuestion('L1', 'q/../x', 'yes');
  assert.equal(calls[0].path, '/goal-loops/L1/questions/q%2F..%2Fx/answer');
});

test('scheduled tasks: the start snapshot landing after the finish one is dropped', async () => {
  const pending: ReturnType<typeof deferred<any>>[] = [];
  const scheduledTasksApi = { runs: () => { const d = deferred<any>(); pending.push(d); return d.promise; } };
  const { scheduledTasks } = loadSource(new URL('../src/lib/stores/scheduledTasks.svelte.ts', import.meta.url), {
    '../api/scheduledTasks': { scheduledTasksApi },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../lazyModule': { announceModule() {} },
    '../latest': latest(),
  });
  const onStart = scheduledTasks.loadRuns('T1');
  const onFinish = scheduledTasks.loadRuns('T1');
  pending[1].resolve([{ id: 'r1', status: 'succeeded' }]);
  await onFinish;
  pending[0].resolve([{ id: 'r1', status: 'running' }]);
  await onStart;
  assert.equal(scheduledTasks.runsByTask.T1[0].status, 'succeeded');
});

test('lazyComponent: a failed prefetch stays silent and the first open retries', async () => {
  const { lazyComponent } = loadSource(new URL('../src/lib/lazy-component.svelte.ts', import.meta.url), {});
  let calls = 0;
  const lc = lazyComponent(() => {
    calls += 1;
    return calls === 1 ? Promise.reject(new Error('offline')) : Promise.resolve({ default: 'Comp' });
  });
  lc.prefetch();
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(lc.error, null, 'a hover prefetch failure is not shown');
  assert.equal(lc.component, null); // starts the real load
  await new Promise((r) => setTimeout(r, 0));
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(calls, 2);
  assert.equal(lc.component, 'Comp');
});
