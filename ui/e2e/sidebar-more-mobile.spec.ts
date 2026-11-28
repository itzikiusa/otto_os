import { test, expect, type Page } from '@playwright/test';

// Phone: the bottom nav follows the sidebar's "show what he uses" rule
// (lib/sidebarMore.ts) — the modules he used lately are the tabs, the rest wait
// in the one drawer behind the "Menu" tab. Real taps.

async function boot(page: Page, route: string, used: string[]): Promise<void> {
  await page.addInitScript((u) => {
    for (const k of ['otto_sidebar_used', 'otto_sidebar_favorites', 'otto_sidebar_hidden', 'otto_sidebar_order', 'otto_light_mode']) localStorage.removeItem(k);
    localStorage.setItem('otto_sidebar_used', JSON.stringify(u));
  }, used);
  await page.goto(`/#/${route}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
}

test('phone: the tabs are what he used; the rest is behind Menu', async ({ page }) => {
  await boot(page, 'agents', ['agents', 'vault', 'kubernetes']);
  test.skip(!(await page.evaluate(() => window.innerWidth <= 640)), 'phone only');
  const bar = page.locator('nav.bottomnav');
  const tabs = await bar.locator('.bn-btn[data-nav-id]').evaluateAll((els) => els.map((e) => e.getAttribute('data-nav-id')));
  expect(tabs.slice(0, 3)).toEqual(['agents', 'vault', 'kubernetes']);
  for (const t of await bar.locator('.bn-btn').all()) {
    const box = (await t.boundingBox())!;
    expect(box.height).toBeGreaterThanOrEqual(44);
  }
  await bar.getByRole('button', { name: /^Menu/ }).tap();
  const sheet = page.locator('.drawer[aria-label="Navigator"]');
  await expect(sheet).toBeVisible();
  const git = sheet.locator('[data-nav-id="git"]').first();
  if (!(await git.isVisible())) await sheet.getByTestId('sidebar-more-toggle').tap();
  await git.tap();
  await expect.poll(() => page.evaluate(() => location.hash)).toMatch(/^#\/git/);
});

// The bar's four are the most used (ranked), not the first four in sidebar
// order: Git and Connections, used most, are tabs ahead of Agents (design H9).
test('phone: the tabs follow how much he uses them, not the sidebar order', async ({ page }) => {
  await boot(page, 'home', ['git', 'connections', 'agents', 'vault', 'home']);
  test.skip(!(await page.evaluate(() => window.innerWidth <= 640)), 'phone only');
  const bar = page.locator('nav.bottomnav');
  const tabs = await bar.locator('.bn-btn[data-nav-id]').evaluateAll((els) => els.map((e) => e.getAttribute('data-nav-id')));
  expect(tabs).toEqual(['git', 'connections', 'agents', 'vault']);
});
