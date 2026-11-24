import { test } from '@playwright/test';
import { LAZY_CASES, expectLazyOnFirstUse, mockMode } from './lazyPages';

// Page diets (effort 2): each heavy view loads on first use behind a calm
// placeholder, never with its page (desktop; the phone twin is
// lazy-pages-mobile.spec.ts).

test.beforeEach(async ({ page }, ti) => {
  test.skip(ti.project.name !== 'desktop-browser', 'desktop-browser only');
  await mockMode(page);
  await page.setViewportSize({ width: 1440, height: 900 });
});

for (const [name, make] of Object.entries(LAZY_CASES)) {
  test(`${name}: the heavy view loads on first use behind a calm placeholder`, async ({ page }) => {
    await expectLazyOnFirstUse(page, make((l) => l.click()));
  });
}
