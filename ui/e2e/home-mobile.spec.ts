import { test, expect } from '@playwright/test';
import { expectNoHorizontalOverflow } from './helpers';

// Home on a phone: the same one column above the bottom nav — greeting, the
// "Ask anything" composer (no in-app floating bar on a phone: it opens the
// command sheet), "Needs you" / "Working" as inbox rows, then Pinned. Real taps.

test('phone: Home is one column; the composer opens the command sheet; rows are finger-sized', async ({ page }) => {
  await page.addInitScript(() => localStorage.removeItem('otto_home_views'));
  await page.goto('/#/home');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  test.skip(!(await page.evaluate(() => window.innerWidth <= 640)), 'phone only');
  const today = page.getByRole('region', { name: 'Today' });
  await expect(today.locator('.hello')).toBeVisible();
  await expect(today.locator('[data-card="needs"]')).toContainText('Needs you');
  await expect(today.locator('[data-card="running"]')).toContainText('Working');
  for (const row of await today.locator('[data-testid^="home-row-"] .tr-main').all()) {
    expect((await row.boundingBox())!.height).toBeGreaterThanOrEqual(44);
  }
  await expectNoHorizontalOverflow(page);
  const ask = page.getByTestId('home-ask');
  expect((await ask.boundingBox())!.height).toBeGreaterThanOrEqual(44);
  await ask.tap();
  await expect(page.getByRole('dialog').first()).toBeVisible();
});
