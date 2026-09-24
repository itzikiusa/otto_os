import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { seedInsights } from './insightsSeed';
import { expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Insights → Reports, against the isolated daemon with synthetic reports
// written straight into its data dir (there is no API to create one — the
// insights skill writes the files):
//   • list/detail opens on the newest report, never an empty pane
//   • key findings parsed from the summary: KPI tiles (the summary's tool-error
//     count beats the collector's 0 in index.json), Action Plan with effort +
//     status chips, ledger detail
//   • period filter; Markdown source; the HTML report in a sandboxed iframe
//     that can't reach the app's storage; a period without HTML
//   • phone: push navigation, no horizontal overflow
// Desktop-only (runs once): one page load per test to keep it light.
// ─────────────────────────────────────────────────────────────────────────────

test.beforeEach(({}, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'Insights spec runs on desktop only');
});

let wsId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  await ctx.dispose();
  seedInsights();
});

test('reports: list/detail, key findings, action plan, filter, markdown and sandboxed HTML', async ({ page }) => {
  test.setTimeout(180_000);
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto.lastSelection.insights');
    localStorage.removeItem('otto.insights.viewMode');
  }, wsId);
  await page.goto('/#/insights');

  const list = page.locator('[data-testid="report-list"]');
  await expect(list).toBeVisible({ timeout: 60_000 });
  await expect(list.locator('button.row')).toHaveCount(13);

  // Opens on the newest report (daily, Wed 23 Sep) — no empty pane.
  const report = page.locator('[data-testid="insight-report"]');
  await expect(report).toBeVisible();
  await expect(list.locator('button.row').first()).toHaveAttribute('aria-current', 'true');
  await expect(report.locator('.r-headline')).toContainText('131 Claude sessions');

  // KPI tiles. Tool errors come from the summary (24), not index.json's 0.
  await expect(page.locator('[data-testid="kpi-sessions"]')).toContainText('131');
  await expect(page.locator('[data-testid="kpi-toolErrors"]')).toContainText('24');
  await expect(page.locator('[data-testid="kpi-spend"]')).toContainText('$33.40');
  await expect(page.locator('[data-testid="kpi-achievement"]')).toContainText('91%');
  // Fewer errors than the day before (41) → an improving delta, with a trend line.
  await expect(page.locator('[data-testid="kpi-toolErrors"] .kpi-delta.good')).toBeVisible();
  await expect(page.locator('[data-testid="kpi-sessions"] svg[role="img"]')).toHaveAttribute('aria-label', /Sessions over \d+ reports/);

  // Action plan: 4 items, effort + status chips, ledger detail on demand.
  const plan = page.locator('[data-testid="action-plan"] > li');
  await expect(plan).toHaveCount(4);
  await expect(plan.first()).toContainText('Hoist build probes');
  await expect(plan.first()).toContainText('Effort M');
  await expect(plan.nth(1).locator('.chip', { hasText: 'Regressed' })).toBeVisible();
  await expect(plan.nth(2).locator('.chip', { hasText: 'New' })).toBeVisible();
  await plan.first().getByRole('button', { name: 'Ledger' }).click();
  await expect(plan.first().locator('.ledger')).toContainText('act-20260914-01');
  await expect(plan.first().locator('.ledger')).toContainText('61%');

  // The rest of the summary is rendered markdown (a real table), not a text wall.
  await expect(page.locator('[data-testid="report-rendered"] table')).toBeVisible();
  await expect(page.locator('[data-testid="report-rendered"]')).not.toContainText('Action Plan');

  // Markdown: the verbatim source.
  await page.getByRole('button', { name: 'Markdown', exact: true }).click();
  await expect(page.locator('[data-testid="report-markdown"]')).toContainText('## Action Plan (carried into the ledger)');

  // HTML: sandboxed (scripts yes, same-origin never) — the report's own
  // script finds localStorage out of reach.
  await page.getByRole('button', { name: 'HTML', exact: true }).click();
  const frame = page.locator('iframe.r-frame');
  await expect(frame).toBeVisible();
  const sandbox = (await frame.getAttribute('sandbox')) ?? '';
  expect(sandbox).toContain('allow-scripts');
  expect(sandbox).not.toContain('allow-same-origin');
  await expect(page.frameLocator('iframe.r-frame').locator('body[data-storage="blocked"]')).toHaveCount(1);
  await expect(page.frameLocator('iframe.r-frame').locator('.bar')).toHaveCount(6);
  await page.getByRole('button', { name: 'Preview', exact: true }).click();

  // Period filter → weekly reports only; a weekly summary with drifted wording
  // still parses ("carried, regressed" → a Regressed chip).
  await page.locator('.filter-chip', { hasText: 'Weekly' }).click();
  await expect(list.locator('button.row')).toHaveCount(3);
  await list.locator('button.row').first().click();
  await expect(page.locator('[data-testid="action-plan"] > li')).toHaveCount(3);
  await expect(page.locator('[data-testid="action-plan"] > li').nth(1).locator('.chip', { hasText: 'Regressed' })).toBeVisible();

  // The oldest daily has no HTML yet: HTML is disabled and says why.
  await page.locator('.filter-chip', { hasText: 'Daily' }).click();
  await list.locator('button.row').last().click();
  const htmlBtn = page.getByRole('button', { name: 'HTML', exact: true });
  await expect(htmlBtn).toBeDisabled();
  await expect(htmlBtn).toHaveAttribute('title', /no HTML report yet/);
});

test('reports on a phone: push navigation with a back button, no horizontal scroll', async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
  await page.goto('/#/insights');
  const list = page.locator('[data-testid="report-list"]');
  await expect(list).toBeVisible({ timeout: 60_000 });
  await expect(page.locator('[data-testid="insight-report"]')).toBeHidden();
  await expectNoHorizontalOverflow(page);
  await list.locator('button.row').first().click();
  await expect(page.locator('[data-testid="insight-report"]')).toBeVisible();
  await expect(list).toBeHidden();
  await expectNoHorizontalOverflow(page);
  await page.getByRole('button', { name: 'Back to reports' }).click();
  await expect(list).toBeVisible();
});
