// Usage store: a range cache a stale response cannot poison, a range switch
// that says it is loading, exports named after the data they hold, a page
// report that stays slim, and a settings save that reports failure.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource, deferred } from './sourceHarness.ts';

type Get = (path: string) => Promise<unknown>;
function store(get: Get, extra: { put?: (p: string, b: unknown) => Promise<unknown>; files?: string[] } = {}) {
  return loadSource(new URL('../src/lib/api/usage.svelte.ts', import.meta.url), {
    './client': { api: { get, put: extra.put } }, '../poll': {}, '../toast.svelte': { toasts: { error() {}, success() {} } },
    '../loadError': { loadErrorText: String },
    '../components/exporters': { exportCsv: (_: unknown, name: string) => extra.files?.push(name), downloadJson: (_: unknown, name: string) => extra.files?.push(name) },
    '../lazyModule': { announceModule() {} },
  }).usage;
}
const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };
const summary = (days: number, tag: string) => ({ days, tag, providers: [], daily: [], sessions: [], models: [], by_kind: [] });

test('a summary that answers after Refresh cannot land in the refreshed range cache', async () => {
  const pending: { path: string; d: ReturnType<typeof deferred<unknown>> }[] = [];
  const usage = store((path) => {
    if (path === '/usage/status') return Promise.resolve({ available: true });
    if (!path.startsWith('/usage/summary')) return Promise.resolve({});
    const d = deferred<unknown>();
    pending.push({ path, d });
    return d.promise;
  });
  usage.status = { available: true };
  const before = usage.setDays(90); // slow 90d in flight…
  const refresh = usage.loadAll(); // …then Refresh empties the cache and asks again
  await flush();
  assert.equal(pending.length, 2);
  pending[1].d.resolve(summary(90, 'refreshed'));
  await refresh;
  pending[0].d.resolve(summary(90, 'before refresh'));
  await before;
  const away = usage.setDays(7);
  await flush();
  pending[2].d.resolve(summary(7, 'seven'));
  await away;
  await usage.setDays(90); // from the cache
  assert.equal(usage.summary.tag, 'refreshed');
});

test('switching to an unseen range says it is loading until it answers', async () => {
  const d = deferred<unknown>();
  const usage = store(() => d.promise);
  usage.status = { available: true };
  const switching = usage.setDays(180);
  assert.equal(usage.loading, true);
  d.resolve(summary(180, 'x'));
  await switching;
  assert.equal(usage.loading, false);
});

test('an export is named after the window its rows are from', () => {
  const files: string[] = [];
  const usage = store(async () => ({}), { files });
  usage.summary = summary(7, 'shown');
  usage.days = 180; // picked, not loaded yet
  usage.exportProvidersCsv();
  usage.exportSummaryJson();
  assert.deepEqual(files, ['otto-usage-providers-7d.csv', 'otto-usage-summary-7d.json']);
});
