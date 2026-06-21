import { test } from '@playwright/test';
import { PAGES, openPage, expectNoHorizontalOverflow, expectAccessible } from './helpers';

// Light-mode coverage: the baseline runs in the default (dark) scheme; this
// forces LIGHT via the persisted scheme key and re-checks every page for
// horizontal overflow + critical a11y (catches light-only contrast/visibility
// regressions). Scoped to a representative phone + desktop profile to keep the
// matrix bounded.
test.beforeEach(async ({ page }, testInfo) => {
  test.skip(
    !['iphone-portrait', 'ipad-landscape'].includes(testInfo.project.name),
    'light-mode matrix scoped to iphone-portrait + ipad-landscape',
  );
  await page.addInitScript(() => {
    localStorage.setItem('otto_scheme', 'light');
  });
});

for (const id of PAGES) {
  test(`light mode — ${id}: no overflow + accessible`, async ({ page }) => {
    await openPage(page, id);
    await expectNoHorizontalOverflow(page);
    await expectAccessible(page);
  });
}
