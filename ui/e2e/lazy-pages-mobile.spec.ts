import { test } from '@playwright/test';
import { LAZY_CASES, expectLazyOnFirstUse, mockMode, productCase, seedStory, seedVault, skillsCase, vaultCase } from './lazyPages';

// Page diets (effort 2) on the phone: the same lazy-on-first-use check as
// desktop-lazy-pages.spec.ts, driven by real taps.

// WebKit routes module fetches through the app's service worker once it has
// claimed the page, where page.route never sees them (see playwright.config).
test.use({ serviceWorkers: 'block' });

test.beforeEach(async ({ page }, ti) => {
  test.skip(!['iphone-portrait', 'android-phone'].includes(ti.project.name), 'iphone-portrait + android-phone');
});

for (const [name, make] of Object.entries(LAZY_CASES)) {
  test(`${name}: the heavy view loads on first use behind a calm placeholder (phone)`, async ({ page }) => {
    await mockMode(page);
    await expectLazyOnFirstUse(page, make((l) => l.tap()));
  });
}

test('vault: the note editor loads on the first Edit (phone)', async ({ page }) => {
  await seedVault(page);
  await expectLazyOnFirstUse(page, vaultCase((l) => l.tap()));
});

test('product: a story tab loads when it is opened (phone)', async ({ page }) => {
  const title = await seedStory(page);
  await expectLazyOnFirstUse(page, productCase((l) => l.tap(), title));
});

test('skills lab: the editor loads on Files (phone)', async ({ page }) => {
  await expectLazyOnFirstUse(page, skillsCase((l) => l.tap()));
});
