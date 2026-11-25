import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

// The cold-mount log prefetch (graph/graphData.svelte.ts): a page asked for
// while the overview is in flight, taken once by the graph. It must not
// outlive its use: a failed read, a switch away or a refs bump drops it.

function load() {
  const calls: { url: string; signal?: AbortSignal; d: ReturnType<typeof deferred<unknown>> }[] = [];
  const timers: (() => void)[] = [];
  let now = 1_000;
  const git = { refsRev: {} as Record<string, number> };
  const mod = loadSource(new URL('../src/modules/git/graph/graphData.svelte.ts', import.meta.url), {
    svelte: { untrack: (f: () => unknown) => f() },
    '../../../lib/api/client': { api: { get: (url: string, signal?: AbortSignal) => { const d = deferred<unknown>(); calls.push({ url, signal, d }); return d.promise; } } },
    '../../../lib/toast.svelte': { toasts: {} },
    '../../../lib/toastError': { toastError() {} },
    '../../../lib/loadError': { loadErrorText: String },
    '../../../lib/stores/git.svelte': { git },
    '../../../lib/api/gitOverview': { readOverview() {} },
    '../../../lib/diff/gitBridge.svelte': { gitBridge: {} },
    '../graph-cache': { graphCache: { get: () => undefined } },
    '../graph-splice': { spliceHistory() {} },
    '../ref-menus': { headShaOf() {}, withoutRemoteHeads: (r: unknown) => r },
    './graphSync.svelte': { createGraphSync() {} },
  }, { Date: { now: () => now }, setTimeout: (f: () => void) => { timers.push(f); return timers.length; }, clearTimeout() {} });
  return { mod, calls, git, timers, advance: (ms: number) => { now += ms; }, flush: async () => { for (let i = 0; i < 5; i++) await Promise.resolve(); } };
}

test('a failed prefetch is dropped: the next open asks again', async () => {
  const { mod, calls, flush } = load();
  mod.prefetchFirstPage('A');
  calls[0].d.reject(new Error('down'));
  await flush();
  mod.prefetchFirstPage('A');
  assert.equal(calls.length, 2);
  assert.equal(mod.takeFirstPage('A').page, calls[1].d.promise);
});

test('a page nobody took (switched away) is aborted and dropped, so a later open prefetches again', () => {
  const { mod, calls, timers, advance } = load();
  mod.prefetchFirstPage('A');
  advance(3_500);
  for (const f of timers) f();
  assert.equal(calls[0].signal?.aborted, true);
  mod.prefetchFirstPage('A');
  assert.equal(calls.length, 2);
});

test('a refs bump after the prefetch flags the page: the graph re-checks its refs after using it', () => {
  const { mod, git } = load();
  mod.prefetchFirstPage('A');
  assert.equal(mod.takeFirstPage('A').moved, false);
  mod.prefetchFirstPage('A');
  git.refsRev.A = 1;
  assert.equal(mod.takeFirstPage('A').moved, true);
  assert.equal(mod.takeFirstPage('A'), null, 'taken once');
});
