import { test, expect } from '@playwright/test';
import { PAGES, openPage, expectNoHorizontalOverflow, expectAccessible } from './helpers';

// RTL coverage: force right-to-left via the persisted direction key and verify
// every page still has no horizontal overflow + no critical a11y, and that the
// document direction actually applied (the layout uses logical CSS properties so
// it mirrors). Scoped to a representative phone + desktop profile.
test.beforeEach(async ({ page }, testInfo) => {
  test.skip(
    !['iphone-portrait', 'ipad-landscape'].includes(testInfo.project.name),
    'RTL matrix scoped to iphone-portrait + ipad-landscape',
  );
  await page.addInitScript(() => {
    localStorage.setItem('otto_direction', 'rtl');
  });
});

for (const id of PAGES) {
  test(`rtl — ${id}: applies dir=rtl, no overflow, accessible`, async ({ page }) => {
    await openPage(page, id);
    // The store applied the direction on boot.
    await expect.poll(() => page.evaluate(() => document.documentElement.dir)).toBe('rtl');
    await expectNoHorizontalOverflow(page);
    await expectAccessible(page);
  });
}
