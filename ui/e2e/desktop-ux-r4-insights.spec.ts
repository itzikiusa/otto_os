import { test, expect, type Page } from '@playwright/test';
import { expectNoHorizontalOverflow } from './helpers';
test.use({ serviceWorkers: 'block', actionTimeout: 10_000 });
test.describe.configure({ timeout: 120_000 });
const report = { kind: 'daily', period_start: '2026-09-24', period_end: '2026-09-24', summary: '# Original report', html_path: '', created_at: '2026-09-25T08:00:00Z' };
async function usageFixture(page: Page) {
  await page.route('**/api/v1/usage/**', r => {
    const path = new URL(r.request().url()).pathname;
    if (path.endsWith('/status')) return r.fulfill({ json: { available: true, enabled: true, retention_days: 180, metrics_interval_secs: 60, usage_rows: 0, metric_rows: 0, disk_bytes: 0 } });
    if (path.endsWith('/summary')) return r.fulfill({ json: { days: 30, total_events: 0, total_input_tokens: 0, total_output_tokens: 0, total_cache_read_tokens: 0, total_cache_write_tokens: 0, total_tokens: 0, total_cost_usd: 0, providers: [], daily: [], sessions: [], by_kind: [] } });
    if (path.endsWith('/budgets')) return r.fulfill({ json: { config: { enforce: false, block_on_exceed: false, window_days: 30, providers: [], workspaces: [] }, window_days: 30, rows: [] } });
    return r.fulfill({ json: [] });
  });
}
test('Insights waits for initial report identity before allowing generation', async ({ page }) => {
  let release!: () => void;
  await usageFixture(page);
  await page.goto('/#/usage');
  await expect(page.getByRole('button', { name: 'Edit budgets', exact: true })).toBeVisible();
  await page.route('**/api/v1/insights/reports', async r => { await new Promise<void>(resolve => { release = resolve; }); return r.fulfill({ json: [report] }); });
  await page.evaluate(() => { location.hash = '#/insights'; });
  await expect.poll(() => !!release).toBe(true);
  await expect(page.getByRole('button', { name: 'Run now', exact: true })).toBeDisabled();
  release();
  await expect(page.getByRole('button', { name: 'Run now', exact: true })).toBeEnabled();
});
test('Usage duplicate scope caps are explained before saving', async ({ page }) => {
  await usageFixture(page); await page.goto('/#/usage');
  await page.getByRole('button', { name: 'Edit budgets', exact: true }).click();
  for (let i = 0; i < 2; i++) {
    await page.getByRole('button', { name: 'Add provider cap' }).click();
    await page.getByRole('combobox', { name: 'Provider', exact: true }).nth(i).selectOption('codex');
    await page.getByLabel('Cap in US dollars').nth(i).fill(String(25 + i));
    await page.getByLabel('Cap in US dollars').nth(i).blur();
  }
  await expect(page.getByRole('button', { name: 'Save budgets', exact: true })).toBeDisabled();
  await expect(page.getByRole('alert')).toContainText('one cap per provider');
  await page.getByRole('button', { name: 'Remove this provider cap' }).last().click();
  await expect(page.getByRole('button', { name: 'Save budgets', exact: true })).toBeEnabled();
});
test('Health support bundle failure leaves download available for retry', async ({ page }) => {
  let fail = true;
  await page.route('**/api/v1/support-bundle', r => r.fulfill(fail ? { status: 503, json: { code: 'upstream', message: 'Synthetic bundle unavailable' } } : { json: { redaction_hits: 0, generated_at: '2026-09-26T00:00:00Z', synthetic: true } }));
  await page.goto('/#/insights/health');
  const download = page.getByRole('button', { name: 'Download support bundle', exact: true });
  await download.click();
  await expect(page.getByText('Couldn’t download the support bundle', { exact: true })).toBeVisible();
  await expect(download).toBeEnabled(); fail = false;
  const pending = page.waitForEvent('download'); await download.click();
  expect((await pending).suggestedFilename()).toMatch(/^otto-support-bundle-/);
});

test('Usage auto-refresh cannot restore an old budget after save completes', async ({ page }) => {
  await usageFixture(page);
  let config = { enforce: false, block_on_exceed: false, window_days: 30, providers: [], workspaces: [] };
  let releasePut!: () => void, releaseGet!: () => void, getCount = 0;
  await page.route('**/api/v1/usage/budgets', async r => {
    if (r.request().method() === 'PUT') {
      const next = r.request().postDataJSON();
      await new Promise<void>(resolve => { releasePut = resolve; }); config = next;
      return r.fulfill({ json: { config, window_days: config.window_days, rows: [] } });
    }
    const snapshot = structuredClone(config);
    if (++getCount > 1) await new Promise<void>(resolve => { releaseGet = resolve; });
    return r.fulfill({ json: { config: snapshot, window_days: snapshot.window_days, rows: [] } });
  });
  await page.goto('/#/usage');
  await page.getByRole('button', { name: 'Edit budgets', exact: true }).click();
  await page.clock.install();
  await page.getByRole('button', { name: 'Auto-refresh', exact: true }).click();
  const days = page.getByLabel('Compare spend over the last'); await days.fill('7');
  await page.getByRole('button', { name: 'Save budgets', exact: true }).click();
  await expect.poll(() => !!releasePut).toBe(true);
  await page.clock.runFor(60_050);
  await expect.poll(() => !!releaseGet).toBe(true);
  releasePut(); await expect(page.getByText('Budgets saved', { exact: true })).toBeVisible();
  releaseGet(); await page.waitForTimeout(150);
  await expect(days).toHaveValue('7');
});

test('Insights report timeout releases generation and retry detects the new result', async ({ page }) => {
  let polls = 0, activeChecks = 0, runs = 0, ready = false;
  // A run that remains alive is distinct from one that disappeared/failed.
  await page.route('**/api/v1/insights/runs/active', r => {
    activeChecks++;
    return r.fulfill({ json: runs > 0 && !ready ? [{ run_id: 'synthetic-run', report_key: 'daily:20260924_20260924', report_revision: 'old', started_at: new Date().toISOString() }] : [] });
  });
  await page.route('**/api/v1/insights/reports', r => { return r.fulfill({ json: [{ ...report, ...(ready ? { summary: '# Ready after retry', created_at: '2026-09-26T09:00:00Z' } : {}) }] }); });
  await page.route('**/api/v1/insights/report-status?*', r => { polls++; return r.fulfill({ json: { report: { ...report, summary: new URL(r.request().url()).searchParams.get('summary') === 'true' ? (ready ? '# Ready after retry' : report.summary) : '', html_path: ready ? '/tmp/synthetic-report.html' : null }, html_revision: ready ? 'new' : 'old' } }); });
  await page.route('**/api/v1/insights/run', r => { runs++; return r.fulfill({ json: { started: true, run_id: 'synthetic-run', report_revision: 'old', report_key: 'daily:20260924_20260924' } }); });
  await page.goto('/#/insights');
  await expect(page.getByTestId('insight-report')).toContainText('Original report');
  await page.clock.install();
  await page.getByRole('button', { name: 'Run now', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Running…', exact: true })).toBeDisabled();
  for (let i = 0; i < 100; i++) {
    const before = polls, checksBefore = activeChecks; await page.clock.runFor(3100);
    await expect.poll(() => polls).toBeGreaterThan(before);
    await expect.poll(() => activeChecks).toBeGreaterThan(checksBefore);
    // Flush the fulfilled response before advancing the next recursive timer.
    await page.waitForTimeout(20);
  }
  await expect(page.getByText('Still generating the insights report', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Run now', exact: true })).toBeEnabled();
  expect(runs).toBe(1);
  await page.getByRole('button', { name: 'Run now', exact: true }).click(); ready = true;
  await expect.poll(() => runs).toBe(2);
  await expect(page.getByRole('button', { name: 'Running…', exact: true })).toBeDisabled();
  await page.clock.runFor(3100);
  await expect(page.getByTestId('insight-report')).toContainText('Ready after retry');
});

for (const [theme, scheme, width, dir] of [['native', 'light', 1440, 'ltr'], ['native', 'dark', 1440, 'ltr'], ['warm', 'dark', 834, 'rtl']] as const) {
  test(`Insights findings fill available width coherently ${theme} ${scheme} ${width}`, async ({ page }, info) => {
    await page.addInitScript(({ theme, scheme, dir }) => { localStorage.setItem('otto_theme', theme); localStorage.setItem('otto_scheme', scheme); localStorage.setItem('otto_direction', dir); }, { theme, scheme, dir });
    await page.setViewportSize({ width, height: 1000 });
    await page.route('**/api/v1/insights/reports', r => r.fulfill({ json: [{ ...report, summary: '# Synthetic review\n\n64 sessions, 142 user turns, 31 tool errors and 78% achievement.\n\n## Action Plan\n\n1. **Verify recovery** — failures: 31 → 10 — effort M — improved\n\n## Notes\n\nUse the report to plan the next review.' }] }));
    await page.goto('/#/insights/r/daily/2026-09-24/2026-09-24');
    const kpis = page.locator('.kpis'); await expect(kpis.locator('.kpi')).toHaveCount(4);
    const rects = await kpis.locator('.kpi').evaluateAll(nodes => nodes.map(n => { const r = n.getBoundingClientRect(); return { top: r.top, width: r.width }; }));
    if (width === 834) { expect(rects[0].top).toBe(rects[1].top); expect(rects[2].top).toBe(rects[3].top); expect(rects[2].top).toBeGreaterThan(rects[0].top); }
    else { expect(new Set(rects.map(r => r.top)).size).toBe(1); }
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath(`report-${theme}-${scheme}-${width}.png`) });
  });
}
