import { test, expect } from '@playwright/test';
import { expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block' });
test.describe.configure({ timeout: 120_000 });

const reports = [
  { kind: 'daily', period_start: '2026-09-24', period_end: '2026-09-24', summary: '# Daily review\n\nDaily headline.', html_path: '', created_at: '2026-09-25T08:00:00Z' },
  { kind: 'weekly', period_start: '2026-09-14', period_end: '2026-09-20', summary: '# Weekly review\n\nWeekly headline.', html_path: '', created_at: '2026-09-21T08:00:00Z' },
];

test('Insights period filter keeps the open report in the visible list', async ({ page }) => {
  let failed = true;
  await page.route('**/api/v1/insights/reports', (route) => route.fulfill(failed ? { status: 503, json: { message: 'Reports temporarily unavailable' } } : { json: reports }));
  await page.goto('/#/insights');
  await expect(page.getByRole('button', { name: 'Retry', exact: true })).toBeVisible();
  failed = false;
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.locator('[data-testid="report-list"] .rep-row')).toHaveCount(2);
  await page.getByRole('button', { name: 'Weekly 1', exact: true }).click();
  await expect(page.locator('[data-testid="report-list"] .rep-row[aria-current="true"]')).toHaveCount(1);
  await expect(page.locator('[data-testid="insight-report"]')).toContainText('Weekly headline.');
  await page.screenshot({ path: '/tmp/otto-ux-screenshots/insights-r1-reports-loaded.png' });
});

test('Usage attribution recovers after failure and fits long keys on phone', async ({ page }) => {
  const tokens = { events: 2, input_tokens: 3000, output_tokens: 1000, cache_read_tokens: 0, cache_write_tokens: 0, total_tokens: 4000, cost_usd: 2.5 };
  let fail = true;
  await page.route('**/api/v1/usage/**', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path.endsWith('/status')) return route.fulfill({ json: { available: true, enabled: true, binary: '/tmp/clickhouse', version: 'test', data_dir: '/tmp/usage', retention_days: 180, metrics_interval_secs: 60, usage_rows: 2, metric_rows: 0, disk_bytes: 1024 } });
    if (path.endsWith('/summary')) return route.fulfill({ json: { days: 30, total_events: 2, total_input_tokens: 3000, total_output_tokens: 1000, total_cache_read_tokens: 0, total_cache_write_tokens: 0, total_tokens: 4000, total_cost_usd: 2.5, providers: [{ provider: 'claude', ...tokens }], daily: [{ day: '2026-09-24', ...tokens }], sessions: [], by_kind: [] } });
    if (path.endsWith('/attribution')) return route.fulfill(fail ? { status: 503, json: { message: 'Attribution unavailable' } } : { json: [{ key: 'long-repository-name-'.repeat(15), cost_usd: 2.5, tokens: 4000, sessions: 2 }] });
    if (path.endsWith('/metrics')) return route.fulfill({ json: [] });
    return route.fallback();
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#/usage');
  const panel = page.locator('.attribution-panel');
  await expect(panel.getByRole('button', { name: 'Retry', exact: true })).toBeVisible();
  fail = false;
  await panel.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(panel.locator('.attr-row')).toHaveCount(1);
  await expectNoHorizontalOverflow(page);
  const download = page.waitForEvent('download');
  await panel.getByRole('button', { name: 'Download attribution as CSV' }).click();
  expect((await download).suggestedFilename()).toBe('otto-attribution-origin-30d.csv');
  await page.screenshot({ path: '/tmp/otto-ux-screenshots/insights-r1-usage-phone-loaded.png' });
});
