import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred, loadSource } from './sourceHarness.ts';
import { computeRunStats, EMPTY_AGENT_STATS } from '../src/modules/swarm/runStats.ts';
function analysis(getAnalysis: (id: string) => Promise<any>) {
  const product = { selectedId: 'story', getAnalysis, captureSelection() {
    const id = this.selectedId; return () => this.selectedId === id;
  } };
  return componentFunctions(new URL('../src/modules/product/AnalysisTab.svelte', import.meta.url),
    ['pollOnce', 'clearPoll', 'isTerminal', 'selectHistory', 'startPolling'], {
      activeId: 'A', activeDetail: null, pollStartedAt: 0, pollTimer: null,
      POLL_MAX_MS: 120_000, POLL_INTERVAL_MS: 3000, viewGeneration: 0, readSequence: 0,
      product, toasts: { warn() { throw new Error('Client must not claim server timed out'); } },
      toastError() {}, liveQuery() { return { stop() {} }; },
    });
}
test('analysis still observes completion after two minutes', async () => {
  const state = analysis(async id => ({ analysis: { id, status: 'done' } }));
  await state.pollOnce(); assert.equal(state.activeDetail.analysis.status, 'done');
});
test('late analysis refresh cannot overwrite a newly selected run', async () => {
  const old = deferred<any>();
  const state = analysis(id => id === 'A' ? old.promise : Promise.resolve({ analysis: { id, status: 'running' } }));
  state.pollStartedAt = Date.now();
  const pending = state.pollOnce(); await state.selectHistory({ id: 'B' });
  old.resolve({ analysis: { id: 'A', status: 'done' } }); await pending;
  assert.equal(state.activeDetail.analysis.id, 'B'); assert.equal(state.activeId, 'B');
});
function swarmFixture(api: Record<string, unknown>) {
  const { swarm } = loadSource(new URL('../src/lib/stores/swarm.svelte.ts', import.meta.url), {
    '../api/client': { api }, '../loadError': { loadErrorText: String },
    '../toastError': { toastError() {} }, '../../modules/swarm/runStats': { computeRunStats, EMPTY_AGENT_STATS },
    '../lazyModule': { announceModule() {} },
  });
  swarm.wsId = 'w'; swarm.detail = { id: 'A' }; return swarm;
}
test('late Swarm run list cannot replace the selected swarm or its loading state', async () => {
  const a = deferred<any>(), b = deferred<any>();
  const swarm = swarmFixture({ get: (url: string) => url.includes('swarm_id=A') ? a.promise : b.promise });
  const old = swarm.loadRuns({ swarm_id: 'A' }); swarm.detail = { id: 'B' };
  const current = swarm.loadRuns({ swarm_id: 'B' }); a.resolve([{ id: 'a', status: 'running' }]); await old;
  assert.equal(swarm.runs.length, 0); assert.equal(swarm.runsLoading, true);
  b.resolve([{ id: 'b', status: 'running' }]); await current; assert.equal(swarm.runs[0].id, 'b');
});
test('a list started before Stop cannot resurrect the stopped run', async () => {
  const list = deferred<any>();
  const swarm = swarmFixture({ get: () => list.promise, post: async () => ({ id: 'r', status: 'stopped' }) });
  swarm.runs = [{ id: 'r', status: 'running' }]; const old = swarm.loadRuns({ swarm_id: 'A' });
  await swarm.stopRun('r'); list.resolve([{ id: 'r', status: 'running' }]); await old;
  assert.equal(swarm.runs[0].status, 'stopped');
});
