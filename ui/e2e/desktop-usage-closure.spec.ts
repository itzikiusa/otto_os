import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

test.use({ serviceWorkers: 'block' });

for (const scheme of ['light', 'dark']) {
  test(`Usage phone chart and rows expose day details and keep costs visible — ${scheme}`, async ({ page }) => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    await ctx.dispose();
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript(({ ws, scheme }) => {
      localStorage.setItem('otto_workspace', ws);
      localStorage.setItem('otto_scheme', scheme);
    }, { ws, scheme });
    const counts = { events: 1, input_tokens: 1000, output_tokens: 500, cache_read_tokens: 200, cache_write_tokens: 100, total_tokens: 1800, cost_usd: 1.25 };
    const daily = Array.from({ length: 30 }, (_, i) => ({ ...counts, day: `2026-09-${String(i + 1).padStart(2, '0')}` }));
    let empty = false;
    await page.route('**/usage/status', r => r.fulfill({ json: { available: true, enabled: true, binary: 'fixture', version: 'fixture', data_dir: '/fixture', retention_days: 90, metrics_interval_secs: 10, usage_rows: 30, metric_rows: 0, disk_bytes: 0 } }));
    await page.route('**/usage/metrics?*', r => r.fulfill({ json: [] }));
    await page.route('**/usage/budgets', r => r.fulfill({ json: { config: { enforce: false, block_on_exceed: false, window_days: 30, workspaces: [], providers: [] }, window_days: 30, rows: [] } }));
    await page.route('**/usage/summary?*', r => r.fulfill({ json: {
      days: 30, total_events: empty ? 0 : 30, total_input_tokens: empty ? 0 : 30000,
      total_output_tokens: empty ? 0 : 15000, total_cache_read_tokens: 0,
      total_cache_write_tokens: 0, total_tokens: empty ? 0 : 54000, total_cost_usd: empty ? 0 : 37.5,
      providers: empty ? [] : [{ ...counts, provider: 'codex' }], daily: empty ? [] : daily,
      sessions: [], by_kind: empty ? [] : [{ ...counts, feature: 'agent', sessions: 1 }],
      models: empty ? [] : [{ ...counts, provider: 'codex', model: 'fixture-model' }],
      daily_models: empty ? [] : daily.map(d => ({ ...d, provider: 'codex', model: 'fixture-model' })), scope: 'all',
    } }));
    await page.goto('/#/usage');
    // Cost per day first, tokens one tap away; the Day picker says the day in words.
    await expect(page.getByRole('heading', { name: 'Cost per day' })).toBeVisible();
    const dailySelect = page.getByLabel('Chart day');
    await page.locator('.daily-svg .bar-hit').first().click();
    await expect(dailySelect).toHaveValue('2026-09-01');
    await page.locator('.daily-svg .bar-hit').nth(1).focus();
    await page.locator('.daily-svg .bar-hit').nth(1).press('Space');
    await expect(dailySelect).toHaveValue('2026-09-02');
    await dailySelect.selectOption('2026-09-01');
    await expect(page.getByTestId('usage-daily-details').getByRole('status')).toContainText('2026-09-01');
    await expect(page.getByTestId('usage-daily-details').getByRole('status')).toContainText('1,800');
    await expect(page.getByTestId('usage-daily-details').getByRole('status')).toContainText('$1.25');
    await page.getByRole('group', { name: 'Chart shows' }).getByRole('button', { name: 'Tokens' }).click();
    await expect(page.getByRole('heading', { name: 'Tokens per day' })).toBeVisible();
    await page.getByTestId('usage-where-model').click();
    const modelSelect = page.getByLabel('Model tokens day');
    await modelSelect.selectOption('2026-09-02');
    await expect(page.getByTestId('usage-model-details').getByRole('status')).toContainText('2026-09-02');
    await expect(page.getByTestId('usage-model-details').getByRole('status')).toContainText('fixture-model');
    // Two-line rows: the cost is on line 1 and never scrolls out of its row.
    for (const cost of await page.locator('.cost-row .bar-cost').all()) {
      await expect(cost).toBeVisible();
      const box = (await cost.boundingBox())!;
      expect(box.x + box.width).toBeLessThanOrEqual(390);
    }
    expect(await page.locator('.cost-row .bar-cost').count()).toBeGreaterThan(0);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    for (const selector of ['.daily-svg', '.model-svg']) {
      const svg = page.locator(selector);
      await expect(svg).toBeVisible();
      {
        const scale = await svg.evaluate(el => (el as SVGSVGElement).getScreenCTM()?.a ?? 0);
        expect(scale).toBeGreaterThanOrEqual(0.99);
      }
    }
    await page.getByTestId('usage-daily-details').scrollIntoViewIfNeeded();
    await page.screenshot({ path: `test-results/usage-phone-${scheme}.png`, fullPage: true });
    empty = true;
    await page.reload();
    await expect(page.getByText('No usage in this window', { exact: true })).toBeVisible();
    await expect(page.getByLabel('Chart day')).toHaveCount(0);
    // Budgets stay reachable in the details card even with nothing to show.
    await expect(page.getByTestId('usage-details')).toContainText('Budgets');
    await page.route('**/usage/report?*', r => r.fulfill({ json: {
      days: 30, otto_only: true, scope: 'all', priced_as_of: 'fixture',
      totals: { ...counts, total_tokens: 0 }, daily: [], sessions: [],
    } }));
    await page.locator('[data-testid="page-header"] .ph-more').click();
    await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Report…' }).click();
    await expect(page.getByTestId('usage-report')).toBeVisible();
    await expect(page.getByTestId('usage-report').getByText('No usage in this window', { exact: true })).toBeVisible();
  });
}
