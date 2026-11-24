import { test, expect, type Page } from '@playwright/test';
import { utimesSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { daemonDataDir, seedInsights } from './insightsSeed';

// Every listed report opens, the oldest as well as the newest: its summary is
// read through GET /insights/report as the sibling `summary-*.md` (0.5.0
// refused anything but `report-*.html` there, so every summary outside the
// three `latest` previews said "You don't have access to this"). A period
// left without its HTML long after its run says "Files missing", not
// "still writing". Desktop + iPhone portrait.

/** Registers the spec in the calling file (desktop-browser and iphone-portrait
 *  each pick up their own file; see playwright.config.ts projects). */
export function everyReportOpensSpec(project: string): void {
  test.beforeEach(({}, testInfo) => {
    test.skip(testInfo.project.name !== project, `runs on ${project}`);
  });

  let wsId = '';

  test.beforeAll(async () => {
    const { ctx, base } = await apiCtx();
    wsId = await seedWorkspace(ctx, base);
    await ctx.dispose();
    seedInsights();
    // The oldest daily has a summary + metrics but no HTML: age it past a run's cap.
    const old = new Date('2026-09-15T08:00:00Z');
    for (const f of ['summary', 'metrics']) {
      const ext = f === 'summary' ? 'md' : 'json';
      utimesSync(join(daemonDataDir(), 'insights', 'daily', `${f}-daily-20260914_20260914.${ext}`), old, old);
    }
  });

  async function openRow(page: Page, row: number, phone: boolean): Promise<void> {
    const list = page.locator('[data-testid="report-list"]');
    await list.locator('button.rep-row').nth(row).click();
    await expect(page.locator('[data-testid="insight-report"]')).toBeVisible();
    if (phone) await expect(list).toBeHidden();
  }

  test('the newest and the oldest report both open their summary', async ({ page }, testInfo) => {
    test.setTimeout(120_000);
    const phone = testInfo.project.name === 'iphone-portrait';
    await page.addInitScript((id) => {
      localStorage.setItem('otto_workspace', id as string);
      localStorage.removeItem('otto.lastSelection.insights');
      localStorage.removeItem('otto.insights.viewMode');
    }, wsId);
    await page.goto('/#/insights');
    const list = page.locator('[data-testid="report-list"]');
    await expect(list).toBeVisible({ timeout: 60_000 });
    const rows = list.locator('button.rep-row');
    await expect(rows).toHaveCount(13);
    const report = page.locator('[data-testid="insight-report"]');
    const refused = page.getByText(/load this report.s summary/);

    // Newest: daily 23 Sep.
    await openRow(page, 0, phone);
    await expect(report.locator('.r-headline')).toContainText('131 Claude sessions');
    await expect(refused).toHaveCount(0);

    // Oldest: weekly 31 Aug–6 Sep — outside the `latest` previews, so its whole
    // summary comes from the sibling .md read.
    if (phone) await page.getByRole('button', { name: 'Back to reports' }).click();
    await page.locator('.filter-chip', { hasText: 'Weekly' }).click();
    await expect(rows).toHaveCount(3);
    await expect(rows.last()).toContainText(/Aug 31|31 Aug/);
    await openRow(page, 2, phone);
    await expect(report.locator('.r-headline')).toContainText('402 sessions');
    await expect(page.locator('[data-testid="action-plan"] > li')).toHaveCount(3);
    await expect(refused).toHaveCount(0);
    await page.getByRole('button', { name: 'Markdown', exact: true }).click();
    await expect(page.locator('[data-testid="report-markdown"]')).toContainText('Replace sleep waits with until-loops');
    await page.getByRole('button', { name: 'HTML', exact: true }).click();
    await expect(page.frameLocator('iframe.r-frame').locator('h1')).toContainText('Weekly insights — 2026-08-31');

    // An old period without HTML says its files are missing.
    if (phone) await page.getByRole('button', { name: 'Back to reports' }).click();
    await page.locator('.filter-chip', { hasText: 'Daily' }).click();
    await expect(rows).toHaveCount(10);
    await expect(rows.last()).toContainText(/Sep 14|14 Sep/);
    await expect(rows.last()).toContainText('Files missing');
  });
}
