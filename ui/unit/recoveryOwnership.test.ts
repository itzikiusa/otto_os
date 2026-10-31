import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';
function analysis(getAnalysis: (id: string) => Promise<any>) {
  const product = { selectedId: 'story', getAnalysis, captureSelection() {
    const id = this.selectedId; return () => this.selectedId === id;
  } };
  return componentFunctions(new URL('../src/modules/product/AnalysisTab.svelte', import.meta.url),
    ['pollOnce', 'clearPoll', 'isTerminal', 'selectHistory', 'selectRun', 'startPolling'], {
      activeId: 'A', activeDetail: null, detailLoading: false, detailError: null, pollStartedAt: 0, pollTimer: null,
      POLL_MAX_MS: 30 * 60_000, POLL_INTERVAL_MS: 3000, viewGeneration: 0, readSequence: 0,
      polling: false, pollFailures: 0, lastPollError: null, pollStopped: null, loadErrorText: String,
      product, toasts: { warn() { throw new Error('Client must not claim server timed out'); } },
      toastError() {}, liveQuery() { return { stop() {} }; },
    });
}
test('analysis still observes completion after two minutes', async () => {
  const state = analysis(async id => ({ analysis: { id, status: 'done' } }));
  state.polling = true; state.pollStartedAt = Date.now() - 2 * 60_000 - 1;
  await state.pollOnce(); assert.equal(state.pollStopped, null); assert.equal(state.activeDetail.analysis.status, 'done');
});
test('late analysis refresh cannot overwrite a newly selected run', async () => {
  const old = deferred<any>();
  const state = analysis(id => id === 'A' ? old.promise : Promise.resolve({ analysis: { id, status: 'running' } }));
  state.pollStartedAt = Date.now();
  const pending = state.pollOnce(); await state.selectHistory({ id: 'B' });
  old.resolve({ analysis: { id: 'A', status: 'done' } }); await pending;
  assert.equal(state.activeDetail.analysis.id, 'B'); assert.equal(state.activeId, 'B');
});
