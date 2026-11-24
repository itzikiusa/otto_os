// Export file names follow the period ON SCREEN (summary.days), not a newer
// period still loading or failed (usage.days) — R2-005.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';

const shown = { days: 7, providers: [{ provider: 'claude' }], models: [], by_kind: [], sessions: [] };

test('the usage page names the summary and daily exports by the period shown', () => {
  const said: string[] = [];
  const usage = { days: 180, summary: shown, exportSummaryJson() {}, exportDailyCsv() {} };
  const toasts = { success: (_t: string, body: string) => said.push(body) };
  const f = componentFunctions(new URL('../src/modules/usage/UsagePage.svelte', import.meta.url), ['exportSummary', 'exportDaily'], { usage, toasts });
  f.exportSummary();
  f.exportDaily();
  assert.deepEqual(said, ['otto-usage-summary-7d.json', 'otto-usage-daily-7d.csv']);
});

test('Where it went names its exports — the attribution CSV too — by the period shown', () => {
  const written: string[] = [];
  const usage = { days: 180, exportProvidersCsv() {} };
  const state = {
    summary: shown, usage, usageUi: { where: 'provider' }, attrRows: [{ key: 'x', cost_usd: 1, tokens: 2, sessions: 1 }], attrDim: 'origin',
    DIM_LABELS: { origin: 'Origin' }, exportCsv: (_rows: unknown, file: string) => written.push(file),
  };
  const f = componentFunctions(new URL('../src/modules/usage/WhereItWent.svelte', import.meta.url), ['exportShown'], state);
  assert.equal(f.exportShown(), 'otto-usage-providers-7d.csv');
  state.usageUi.where = 'work';
  assert.equal(f.exportShown(), 'otto-attribution-origin-7d.csv');
  assert.deepEqual(written, ['otto-attribution-origin-7d.csv']);
});
