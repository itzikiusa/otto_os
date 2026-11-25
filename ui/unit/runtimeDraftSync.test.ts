// A held thread's draft re-reads its revision after a turn, once per burst of
// turn events, not once per event (lib/stores/runtimeDraftRecovery.svelte.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

test('a burst of turn events costs one GET /draft, after it settles', async () => {
  let gets = 0;
  let onEvent: ((ev: { event: unknown }) => void) | null = null;
  let init: ((id: string, draft: Record<string, unknown>) => { activate(): () => void }) | null = null;
  const timers = new Map<number, () => void>();
  let tid = 0;
  const persistence = loadSource(new URL('../src/lib/stores/runtimeDraftPersistence.ts', import.meta.url), {});
  const m = loadSource(new URL('../src/lib/stores/runtimeDraftRecovery.svelte.ts', import.meta.url), {
    svelte: { untrack: (f: () => unknown) => f() },
    '../api/client': { api: { get: async () => (gets++, { revision: 1, draft: null }), put: async () => ({ revision: 2, draft: null }) } },
    '../live': { appLive: { on: (_: string[], fn: typeof onEvent) => ((onEvent = fn), () => {}), onResync: () => () => {}, onConnection: () => () => {} } },
    './runtimeDrafts.svelte': { runtimeDrafts: { configure: (fn: typeof init) => (init = fn), epoch: 0, owns: () => true, held: () => true } },
    './runtimeDraftPersistence': persistence,
  }, {
    $effect: Object.assign(() => {}, { root: () => () => {} }),
    setTimeout: (fn: () => void) => (timers.set(++tid, fn), tid),
    clearTimeout: (k: number) => void timers.delete(k),
  });
  m.installRuntimeDraftRecovery();
  const draft = { text: '', images: [], context: [], model: undefined, effort: null, pickedMode: null, freshContext: false, provider: null };
  const root = init!('t1', draft);
  await new Promise((r) => setImmediate(r));
  assert.equal(gets, 1, 'the first load');
  root.activate();
  await new Promise((r) => setImmediate(r));
  const base = gets;
  for (const type of ['provider_session.updated', 'provider_turn.updated', 'queue.updated', 'provider_turn.updated', 'turn.terminal']) {
    onEvent!({ event: { session_id: 't1', type } });
  }
  await new Promise((r) => setImmediate(r));
  assert.equal(gets, base, 'nothing during the burst');
  for (const fn of [...timers.values()]) fn();
  await new Promise((r) => setImmediate(r));
  assert.equal(gets, base + 1, 'one re-read after it');
});
