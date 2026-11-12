import { test, expect, type Page } from '@playwright/test';
import { openPage, expectNoHorizontalOverflow } from './helpers';

// Settings on a phone (≤640 px) is a two-level list, like iOS Settings:
//   • #/settings shows the filter + the grouped sections (each row tagged with
//     who it applies to), never one unlabelled scrolling row of pills;
//   • the filter finds a section by one of its SETTINGS and names the match;
//   • a row opens the section full-width with a back control and its title;
//   • back returns to the list; a deep link opens the section directly.

test.beforeEach(async ({ page }) => {
  test.skip((page.viewportSize()?.width ?? 0) > 640, 'phone-width layout only');
});

const nav = (page: Page) => page.getByRole('navigation', { name: 'Settings sections' });
const filter = (page: Page) => page.getByRole('searchbox', { name: 'Filter settings' });
const title = (page: Page) => page.locator('.settings-body h1.ph-title');
const back = (page: Page) => page.getByRole('button', { name: 'Back to Settings' });

test('list → filter → section → back, and deep links open the section', async ({ page }) => {
  await openPage(page, 'settings');

  // Level one: grouped list with headings and scope tags; no section body.
  await expect(nav(page)).toBeVisible();
  await expect(nav(page).locator('.settings-nav-heading').first()).toBeVisible();
  await expect(nav(page).locator('.settings-nav-scope').first()).toBeVisible();
  await expect(page.locator('.settings-body')).toBeHidden();
  await expectNoHorizontalOverflow(page);

  // The filter matches an individual setting and shows which one.
  await filter(page).fill('font');
  const row = nav(page).getByRole('button', { name: /^Appearance/ });
  await expect(row).toBeVisible();
  await expect(row.locator('.settings-nav-match')).toHaveText('Terminal font');

  // Level two: the section, full width, titled, with a way back.
  await row.click();
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/settings/appearance');
  await expect(title(page)).toHaveText('Appearance');
  await expect(back(page)).toBeVisible();
  await expect(nav(page)).toBeHidden();
  await expectNoHorizontalOverflow(page);

  await back(page).click();
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/settings');
  await expect(filter(page)).toBeVisible();
  await filter(page).fill('');
  await expect(nav(page).locator('.settings-nav-heading').first()).toBeVisible();

  // A deep link skips the list.
  await openPage(page, 'settings/notifications');
  await expect(title(page)).toHaveText('Notifications');
  await expect(back(page)).toBeVisible();
  await expect(nav(page)).toBeHidden();
});
