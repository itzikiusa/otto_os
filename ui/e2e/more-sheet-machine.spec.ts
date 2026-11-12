import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport } from './helpers';

// On a phone the status bar (and its machine chip) is hidden, so the machine
// status lives in the bottom nav's "More" sheet: one full-width row that
// opens the same details dialog — tap, never hover.

test.use({ serviceWorkers: 'block' });

test('the More sheet shows the machine row and opens its details', async ({ page }, info) => {
  test.skip(info.project.name.startsWith('desktop'), 'phone layout only');
  const { ctx, base } = await apiCtx();
  const id = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id), id);
  await page.goto('/#/home');
  // A phone on its side is wide enough for the tablet layout: no bottom nav,
  // and the status bar with its chip is on screen.
  if (info.project.name.includes('landscape')) {
    const chip = page.getByTestId('statusbar-machine');
    await expect(chip).toBeVisible({ timeout: 20_000 });
    await chip.click();
    await expect(page.getByTestId('machine-details')).toBeVisible();
    return;
  }
  await page.locator('.bottomnav').getByRole('button', { name: /^More/ }).click();
  const row = page.getByTestId('more-machine');
  await expect(row).toBeVisible({ timeout: 20_000 });
  await expect(row).toContainText('Machine');
  await expectFullyInViewport(page, row, 'the machine row');
  // The whole row is the tap target (the chip's button covers it).
  const box = (await row.boundingBox())!;
  await page.mouse.click(box.x + 20, box.y + box.height / 2);
  await expect(page.getByTestId('machine-details')).toBeVisible();
});
