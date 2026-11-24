import { test } from '@playwright/test';
import { LAZY_CASES, expectLazyOnFirstUse, mockMode } from './lazyPages';

// Page diets (effort 2) on the phone: the same lazy-on-first-use check as
// desktop-lazy-pages.spec.ts, driven by real taps.

test.beforeEach(async ({ page }, ti) => {
  test.skip(!['iphone-portrait', 'android-phone'].includes(ti.project.name), 'iphone-portrait + android-phone');
  await mockMode(page);
});

for (const [name, make] of Object.entries(LAZY_CASES)) {
  test(`${name}: the heavy view loads on first use behind a calm placeholder (phone)`, async ({ page }) => {
    await expectLazyOnFirstUse(page, make((l) => l.tap()));
  });
}
