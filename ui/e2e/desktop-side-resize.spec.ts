import { test, expect, type Page } from '@playwright/test';

// Resizing across the phone breakpoint with a page already open must give the
// same chrome as loading the page on a phone: ONE header row (design.md §7.1 —
// the page's PageHeader IS the top bar, the shell bar only for pages without
// one) and a Back that returns to the list. The shell used to decide "the page
// draws its own header" from a class the compact layout adds to the centre
// column in the same update, so after the resize it kept its own bar above the
// page's (two rows) until the page swapped headers.

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'resizes a desktop window to phone width');
});

const shellBar = (page: Page) => page.locator('header.mtopbar');
const phoneNav = (page: Page) => page.locator('.shell.bnav');

async function toPhone(page: Page): Promise<void> {
  await page.setViewportSize({ width: 375, height: 812 });
  // The shell switches on the matchMedia change, a moment after the resize.
  await expect(phoneNav(page)).toBeVisible();
}

test('a nested Settings section opened on desktop keeps one header row and its Back at phone width', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#/settings/access-groups');
  await expect(page.locator('.settings h1.ph-title').first()).toBeVisible();
  await toPhone(page);
  const back = page.getByRole('button', { name: 'Back to Settings' });
  await expect(back).toBeVisible();
  await expect(shellBar(page)).toHaveCount(0);
  await back.click();
  await expect(page.getByRole('searchbox', { name: 'Filter settings' })).toBeVisible();
  await expect(shellBar(page)).toHaveCount(0);
  // …and a section opened from the list after the resize pushes again.
  await page.getByRole('searchbox', { name: 'Filter settings' }).fill('Sessions');
  await page.locator('.settings-nav').getByRole('button', { name: /^Sessions/ }).first().click();
  await expect(back).toBeVisible();
  await expect(shellBar(page)).toHaveCount(0);
  await back.click();
  await expect(page.getByRole('searchbox', { name: 'Filter settings' })).toBeVisible();
});

for (const route of ['threads', 'agents', 'git']) {
  test(`#/${route} opened on desktop shows the same top chrome at phone width as when loaded there`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto(`/#/${route}`);
    await expect(page.locator('.center').first()).toBeVisible();
    await page.waitForLoadState('networkidle');
    await toPhone(page);
    const pageHeaders = page.locator('.mcenter [data-testid=page-header]:visible');
    const resized = { bar: await shellBar(page).count(), headers: await pageHeaders.count() };
    await page.reload();
    await expect(phoneNav(page)).toBeVisible();
    await page.waitForLoadState('networkidle');
    const loaded = { bar: await shellBar(page).count(), headers: await pageHeaders.count() };
    expect(resized).toEqual(loaded);
    // Never the shell bar AND a page header: one row.
    expect(resized.bar + Math.min(resized.headers, 1)).toBe(1);
  });
}
