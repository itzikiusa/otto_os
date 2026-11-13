import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// The intent-first Automate chooser: on an EMPTY Automate page it is shown
// open (not a collapsed disclosure) with one row per place — Scheduled tasks,
// Workflows, Goal Loops, Personal Agents — and each row goes there.

let ws = '';

test.beforeAll(async ({}, testInfo) => {
  if (testInfo.project.name !== 'desktop-browser') return;
  const a = await apiCtx();
  ws = await seedWorkspace(a.ctx, a.base, 'E2E Automate Chooser');
  await a.ctx.dispose();
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);
});

test('an empty Goal Loops page shows the chooser open, and a row opens its place', async ({ page }) => {
  await page.goto('/#/loops');
  const guide = page.getByTestId('automate-guide');
  await expect(guide).toBeVisible({ timeout: 15_000 });
  // Open: a section, not a collapsed <details>.
  await expect(guide).not.toHaveJSProperty('tagName', 'DETAILS');
  const rows = guide.getByTestId('automate-guide-row');
  await expect(rows).toHaveCount(4);
  await expect(rows.nth(0)).toContainText('Run one prompt or command on a schedule');
  await expect(guide.locator('[aria-current="page"]')).toContainText('Goal Loops');
  await expect(guide).toContainText('Every morning? One prompt → a Scheduled task.');

  await guide.getByRole('button', { name: /Scheduled tasks/ }).click();
  await expect(page).toHaveURL(/#\/scheduled-tasks$/);
});

test('the chooser fits a 390 px phone width', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#/loops');
  const guide = page.getByTestId('automate-guide');
  await expect(guide).toBeVisible({ timeout: 15_000 });
  await expect(guide.getByTestId('automate-guide-row')).toHaveCount(4);
  await expectNoHorizontalOverflow(page);
});
