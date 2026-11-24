// Insights: a report summary that cannot be read shows why, with Retry —
// it used to leave a blank preview (the store swallowed the error).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function store(readText: (p: string) => Promise<string>) {
  return loadSource(
    new URL('../src/modules/insights/insightsStore.svelte.ts', import.meta.url),
    {
      '../../lib/api/insights': { insightsApi: { readText } },
      '../../lib/confirm.svelte': { confirmer: {} },
      '../../lib/toast.svelte': { toasts: {} },
      '../../lib/toastError': { toastError: () => {} },
      './capabilities': { capabilitiesApi: {} },
      './runPoll': { runPollDelay: () => 0 },
    },
    {},
    { relative: true },
  ).insightsStore;
}

const report = { kind: 'daily', period_start: '2026-11-23', period_end: '2026-11-23', html_path: '/r/report-daily-2026-11-23_2026-11-23.html', summary: '' };
const flush = () => new Promise((r) => setTimeout(r, 0));

test('a failed summary read records why; Retry reads it again', async () => {
  let fail = true;
  const s = store(async () => {
    if (fail) throw new Error('permission denied');
    return '# Insights summary';
  });
  s.loadSummaries([report]);
  await flush();
  const [key] = Object.keys(s.summaryError);
  assert.ok(key, 'the failure is recorded');
  assert.match(s.summaryError[key], /permission denied/);
  fail = false;
  s.retrySummary(report);
  await flush();
  assert.deepEqual(Object.keys(s.summaryError), []);
  assert.equal(s.fullMd[key], '# Insights summary');
});
