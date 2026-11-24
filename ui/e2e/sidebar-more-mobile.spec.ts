import { test, expect, type Page } from '@playwright/test';

// Phone: the bottom nav follows the sidebar's "show what he uses" rule
// (lib/sidebarMore.ts) — the modules he used lately are the tabs, the rest wait
// in the More sheet. Real taps.

async function boot(page: Page, route: string, used: string[]): Promise<void> {
  await page.addInitScript((u) => {
    for (const k of ['otto_sidebar_used', 'otto_sidebar_favorites', 'otto_sidebar_hidden', 'otto_sidebar_order', 'otto_light_mode']) localStorage.removeItem(k);
    localStorage.setItem('otto_sidebar_used', JSON.stringify(u));
  }, used);
  await page.goto(`/#/${route}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
}

test('phone: the tabs are what he used; the rest is in More', async ({ page }) => {
  await boot(page, 'agents', ['agents', 'vault', 'kubernetes']);
  test.skip(!(await page.evaluate(() => window.innerWidth <= 640)), 'phone only');
  const bar = page.locator('nav.bottomnav');
  const tabs = await bar.locator('.bn-btn[data-nav-id]').evaluateAll((els) => els.map((e) => e.getAttribute('data-nav-id')));
  expect(tabs.slice(0, 3)).toEqual(['agents', 'vault', 'kubernetes']);
  for (const t of await bar.locator('.bn-btn').all()) {
    const box = (await t.boundingBox())!;
    expect(box.height).toBeGreaterThanOrEqual(44);
  }
  await bar.getByRole('button', { name: /^More/ }).tap();
  const sheet = page.getByRole('dialog', { name: 'More' });
  await expect(sheet).toBeVisible();
  await sheet.getByRole('button', { name: 'Git', exact: true }).tap();
  await expect.poll(() => page.evaluate(() => location.hash)).toMatch(/^#\/git/);
});
