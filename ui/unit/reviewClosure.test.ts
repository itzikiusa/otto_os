import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';

const findingPath = new URL('../src/modules/git/FindingActions.svelte', import.meta.url);
for (const kind of ['waive', 'false_positive']) {
  test(`${kind} records the supplied reason and cancelling changes nothing`, async () => {
    const calls: unknown[][] = [];
    let answer: string | null = null;
    const c = componentFunctions(findingPath, ['resolveFinding', 'run'], {
      busy: '', finding: { id: 'A', title: 'Finding A', body: 'Evidence' },
      confirmer: { promptText: async () => answer },
      waiveFinding: async (...args: unknown[]) => { calls.push(args); return { id: 'A' }; },
      falsePositiveFinding: async (...args: unknown[]) => { calls.push(args); return { id: 'A' }; },
      onupdated() {}, toasts: { success() {} }, toastError() {},
    });
    await c.resolveFinding(kind);
    assert.equal(calls.length, 0);
    answer = 'Checked against the current requirements';
    await c.resolveFinding(kind);
    assert.deepEqual(calls, [['A', answer]]);
  });
}
test('a finding changed while the reason dialog is open is not resolved', async () => {
  const prompt = deferred<string>(); let calls = 0;
  const c = componentFunctions(findingPath, ['resolveFinding'], {
    busy: '', finding: { id: 'A', title: 'A' }, confirmer: { promptText: () => prompt.promise },
    run: async () => { calls++; },
  });
  const pending = c.resolveFinding('waive');
  c.finding = { id: 'B', title: 'B' };
  prompt.resolve('Reason for A'); await pending;
  assert.equal(calls, 0);
});

test('failed proof loading disables export, retry succeeds, stale loads cannot replace it', async () => {
  const responses = [deferred<unknown>(), deferred<unknown>(), deferred<unknown>()];
  let next = 0, exports = 0;
  const c = componentFunctions(new URL('../src/modules/git/ProofPackView.svelte', import.meta.url), ['load', 'doExport'], {
    loadGeneration: 0, loading: false, loadErr: '', pack: null, exportedMd: '', exporting: false, reviewId: 'A',
    getProofPack: () => responses[next++].promise,
    exportProofPack: async () => { exports++; return { markdown: 'Verified report' }; },
    loadErrorText: (e: Error) => e.message, toasts: { success() {} }, toastError() {},
  });
  const failed = c.load('A'); responses[0].reject(new Error('Unavailable')); await failed;
  assert.equal(c.loadErr, 'Unavailable'); await c.doExport(); assert.equal(exports, 0);
  const old = c.load('A'), retry = c.load('A');
  responses[2].resolve({ id: 'fresh' }); await retry;
  responses[1].resolve({ id: 'obsolete' }); await old;
  assert.equal(c.pack.id, 'fresh'); assert.equal(c.loadErr, ''); assert.equal(c.loading, false);
  await c.doExport(); assert.equal(exports, 1);
});

for (const staleFails of [false, true]) {
  test(`merge readiness preserves newer data after stale ${staleFails ? 'failure' : 'success'}`, async () => {
    const old = deferred<unknown>(); let calls = 0;
    const c = componentFunctions(new URL('../src/modules/git/ReviewPanel.svelte', import.meta.url), ['loadFindingsAndReadiness'], {
      reviewGeneration: 1, readinessRequest: 0, disposed: false, review: { id: 'A' },
      mergeReadiness: null, readinessError: null, findings: [], findingsLoading: false, mergeReadinessLoading: false,
      api: { get: () => { calls++; return calls <= 2 ? old.promise : Promise.resolve({ fresh: true }); } },
      loadErrorText: (e: Error) => e.message,
    });
    const first = c.loadFindingsAndReadiness('A'); await c.loadFindingsAndReadiness('A');
    if (staleFails) old.reject(new Error('Old failure')); else old.resolve({ fresh: false });
    await first;
    assert.equal(c.mergeReadiness.fresh, true); assert.equal(c.readinessError, null);
    assert.equal(c.mergeReadinessLoading, false);
  });
}
test('readiness load failure is visible and a retry clears it', async () => {
  let fail = true;
  const c = componentFunctions(new URL('../src/modules/git/ReviewPanel.svelte', import.meta.url), ['loadFindingsAndReadiness'], {
    reviewGeneration: 1, readinessRequest: 0, disposed: false, review: { id: 'A' },
    mergeReadiness: null, readinessError: null, findings: [], findingsLoading: false, mergeReadinessLoading: false,
    api: { get: async () => { if (fail) throw new Error('Service unavailable'); return {}; } },
    loadErrorText: (e: Error) => e.message,
  });
  await c.loadFindingsAndReadiness('A'); assert.equal(c.readinessError, 'Service unavailable');
  fail = false; await c.loadFindingsAndReadiness('A'); assert.equal(c.readinessError, null);
});
