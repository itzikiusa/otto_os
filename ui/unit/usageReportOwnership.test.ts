import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource, deferred } from './sourceHarness.ts';

function store(get: (path: string) => Promise<unknown>) {
  return loadSource(new URL('../src/lib/api/usage.svelte.ts', import.meta.url), {
    './client': { api: { get } }, '../poll': {}, '../toast.svelte': { toasts: {} },
    '../loadError': { loadErrorText: String }, '../components/exporters': {},
    '../lazyModule': { announceModule() {} },
  }).usage;
}

test('failed report refresh cannot export a previous window as the current report', async () => {
  const usage = store(async () => { throw new Error('offline'); });
  usage.report = { days: 30, otto_only: true }; usage.reportFull = true;
  usage.days = 7;
  await usage.loadReport(true);
  assert.equal(await usage.fullReport(), null);
});

test('a report awaiting replacement cannot export the prior session scope', async () => {
  const pending = deferred<unknown>();
  const usage = store(() => pending.promise);
  usage.report = { days: 30, otto_only: false }; usage.reportFull = true;
  usage.ottoOnly = true;
  const loading = usage.loadReport(true);
  const exporting = usage.fullReport();
  pending.resolve({ days: 30, otto_only: true });
  await loading;
  assert.equal((await exporting)?.otto_only, true);
});
