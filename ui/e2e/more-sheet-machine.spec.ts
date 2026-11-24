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
  // A phone on its side (short touch screen): no bottom nav and no status
  // bar — the Navigator drawer's footer carries the machine chip.
  if (['iphone-landscape', 'android-phone-landscape'].includes(info.project.name)) {
    await page.getByRole('button', { name: 'Open navigator' }).click();
    const chip = page.getByTestId('short-nav-extras').getByTestId('statusbar-machine');
    await expect(chip).toBeVisible({ timeout: 20_000 });
    await expectFullyInViewport(page, chip, 'the drawer machine chip');
    await chip.click();
    await expect(page.getByTestId('machine-details')).toBeVisible();
    return;
  }
  // An iPad on its side is a tablet: the status bar with its chip is on screen.
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

// A phone on its side with the browser's bars showing leaves ~300 px: the
// drawer scrolls as a whole — every module reachable, its footer never drawn
// over the extras below it (on the Android emulator the module list got 8 px).
test('a phone on its side: the navigator drawer scrolls and nothing overlaps', async ({ page }, info) => {
  test.skip(info.project.name !== 'android-phone-landscape', 'Android phone landscape');
  await page.setViewportSize({ width: 864, height: 304 });
  await page.goto('/#/home');
  await page.getByRole('button', { name: 'Open navigator' }).click();
  const extras = page.getByTestId('short-nav-extras');
  await expect(extras).toBeAttached({ timeout: 20_000 });
  const nav = (await page.locator('.sne-drawer .sne-nav').boundingBox())!;
  expect(nav.y + nav.height).toBeLessThanOrEqual((await extras.boundingBox())!.y + 1);
  expect((await page.locator('.sne-drawer .nav-scroll').boundingBox())!.height).toBeGreaterThan(100);
  const git = page.locator('.sne-drawer .nav-section.modules').getByRole('button', { name: /^Git/ }).first();
  await git.scrollIntoViewIfNeeded();
  await expect(git).toBeInViewport();
});
