import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

test.use({ serviceWorkers: 'block' });

test('Insights phone loading stays on the list and period filters have touch targets', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript(ws => localStorage.setItem('otto_workspace', ws), ws);
  let release!: () => void;
  const pending = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/insights/reports', async r => {
    await pending;
    await r.fulfill({ json: [{ kind: 'daily', period_start: '2026-10-17', period_end: '2026-10-17', summary: '# Report', html_path: '', created_at: '2026-10-18T08:00:00Z' }] });
  });
  await page.route('**/insights/runs/active', r => r.fulfill({ json: [] }));
  await page.goto('/#/insights');
  try {
    // The list pane holds the loading skeleton (a status named after what
    // loads; no aria-busy, which would defer its announcement).
    await expect(page.locator('.list-pane').getByRole('status', { name: 'Loading insight reports' })).toBeVisible();
    await expect(page.locator('.detail-pane')).not.toBeVisible();
  } finally { release(); }
  await expect(page.getByTestId('report-list')).toBeVisible();
  for (const button of await page.getByRole('group', { name: 'Filter by period' }).getByRole('button').all()) {
    const box = await button.boundingBox();
    expect(box?.height).toBeGreaterThanOrEqual(36);
  }
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('Insights elapsed time catches up without consuming polling attempts', async ({ page }) => {
  const current = new Date('2026-10-18T12:00:00Z');
  await page.clock.install({ time: current });
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript(ws => localStorage.setItem('otto_workspace', ws), ws);
  await page.route('**/insights/reports', r => r.fulfill({ json: [] }));
  await page.route('**/insights/runs/active', r => r.fulfill({ json: [{ run_id: 'fixture-run', report_key: 'daily:20261017_20261017', report_revision: null, started_at: '2026-10-18T11:50:00Z' }] }));
  await page.goto('/#/insights');
  await expect(page.getByText(/10:0\d elapsed/)).toBeVisible();
  // A background interval can be throttled for minutes. Change wall time
  // without firing the polling timer, then deliver visibility restoration.
  await page.clock.setSystemTime(new Date('2026-10-18T12:10:00Z'));
  await page.evaluate(() => document.dispatchEvent(new Event('visibilitychange')));
  await expect(page.getByText(/20:0\d elapsed/)).toBeVisible();
});

test('failed Insights refresh leaves the old report visible with Retry', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript(ws => localStorage.setItem('otto_workspace', ws), ws);
  const report = { kind: 'daily', period_start: '2026-10-17', period_end: '2026-10-17', summary: '# Retained report', html_path: '', created_at: '2026-10-18T08:00:00Z' };
  let reads = 0, activeReads = 0, recover = false;
  await page.route('**/insights/reports', r => ++reads === 1 || recover
    ? r.fulfill({ json: [report] })
    : r.fulfill({ status: 503, json: { message: 'Reports unavailable' } }));
  await page.route('**/insights/runs/active', r => r.fulfill({ json: ++activeReads === 1 ? [{ run_id: 'fixture-run', report_key: 'daily:20261017_20261017', report_revision: null, started_at: new Date().toISOString() }] : [] }));
  await page.route('**/insights/report-status?*', r => r.fulfill({ json: { report, html_revision: null } }));
  await page.goto('/#/insights');
  await expect(page.getByTestId('report-list')).toBeVisible();
  const retry = page.getByRole('button', { name: 'Retry', exact: true });
  await expect(retry).toBeVisible({ timeout: 15000 });
  await expect(page.getByTestId('report-list')).toBeVisible();
  recover = true;
  await retry.click();
  await expect(retry).not.toBeVisible();
  await expect(page.getByTestId('report-list')).toBeVisible();
});
