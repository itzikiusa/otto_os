// Usage, first screen in words (spec E): what is left, "This month ≈ $X · on
// track for $Y", one chart (cost first), one "Where it went" table with a
// switch and one Export; the forecast, attribution and system metrics are
// fetched only when their view opens. Canned data (usage-page-fixture.ts).
import { test, expect } from '@playwright/test';
import { routeUsage } from './usage-page-fixture';
import { expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block' });

const opened = (calls: string[]) => calls.filter((c) => !c.includes('/usage/limits')).map((c) => c.split(' ')[1]).sort();

test('opening Usage asks for four things and says the month in words', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const r = await routeUsage(page, { budgetRows: [{ scope: 'provider', key: 'claude', label: 'Claude', limit_usd: 500, spent_usd: 460, used_fraction: 0.92, warning: true, exceeded: false }] });
  await page.goto('/#/usage');
  const spend = page.getByTestId('usage-spend');
  await expect(spend).toContainText(/This month ≈ \$[\d,.]+ · on track for \$[\d,.]+/);
  await expect(spend).toContainText('Last 30 days ≈ $854.23');
  await expect(spend).toContainText('Claude budget 92 % used');
  await expect(page.getByRole('heading', { name: 'Cost per day' })).toBeVisible();
  // Where it went: provider first, biggest cost on top, cost on the row.
  const where = page.getByTestId('usage-where');
  await expect(where.locator('.cost-row').first()).toContainText('Claude Code');
  await expect(where.locator('.cost-row').first()).toContainText('$529.62');
  await expect(page.getByTestId('usage-details')).toContainText('$460.00 of $500.00');
  // Nothing below the fold was fetched: no metrics, forecast or attribution.
  await expect.poll(() => opened(r.calls)).toEqual(['/usage/budgets', '/usage/status', '/usage/summary']);
  expect(r.calls.some((c) => c.includes('/usage/limits'))).toBe(true);
  await expectNoHorizontalOverflow(page);
});

test('each view fetches its own data only when it opens, and Export saves what is shown', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const r = await routeUsage(page);
  await page.goto('/#/usage');
  await expect(page.getByTestId('usage-where')).toBeVisible();

  let download = page.waitForEvent('download');
  await page.getByTestId('usage-export').click();
  expect((await download).suggestedFilename()).toBe('otto-usage-providers-30d.csv');

  await page.getByTestId('usage-where-feature').click();
  await expect(page.getByTestId('usage-where')).toContainText('Goal loop');
  await expect(page.getByTestId('usage-where')).toContainText('Next agent run on Claude Code ≈ $0.42');
  expect(r.calls).toContain('POST /usage/forecast');

  await page.getByTestId('usage-where-session').click();
  await expect(page.getByRole('button', { name: 'Open session Fix flaky clipboard tests 1', exact: true })).toBeVisible();
  download = page.waitForEvent('download');
  await page.getByTestId('usage-export').click();
  expect((await download).suggestedFilename()).toBe('otto-usage-sessions-30d.csv');

  expect(r.calls.some((c) => c.includes('/usage/attribution'))).toBe(false);
  await page.getByTestId('usage-where-work').click();
  await expect(page.locator('.attribution-panel .attr-row')).toHaveCount(2);
  download = page.waitForEvent('download');
  await page.getByTestId('usage-export').click();
  expect((await download).suggestedFilename()).toBe('otto-attribution-origin-30d.csv');

  expect(r.calls.some((c) => c.includes('/usage/metrics'))).toBe(false);
  await page.getByTestId('usage-system-toggle').click();
  await expect(page.getByTestId('usage-system-load')).toContainText('CPU');
  expect(r.calls).toContain('GET /usage/metrics');
});

test('Report and Diagnostics stay one visible click away', async ({ page }) => {
  await routeUsage(page);
  await page.goto('/#/usage');
  await page.getByTestId('usage-diagnostics').click();
  await expect(page).toHaveURL(/#\/usage\/otto/);
  await page.locator('[data-testid="page-header"]').getByRole('button', { name: 'Usage', exact: true }).click();
  await expect(page.getByTestId('usage-spend')).toBeVisible();
  await page.locator('[data-testid="page-header"] .ph-more').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Report…' }).click();
  await expect(page.getByTestId('usage-report')).toBeVisible();
});
