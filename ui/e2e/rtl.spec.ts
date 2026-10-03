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

// Directional icons mirror in ONE place (Icon.svelte's DIRECTIONAL set), so a
// back chevron points toward inline-start in RTL — exactly once (a leftover
// per-module `scaleX(-1)` override on a wrapper would cancel it out visually).
// Non-directional glyphs stay put. The phone top bar always renders NavButtons.
test('rtl — directional icons mirror once, others do not', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'iphone-portrait', 'phone top bar carries NavButtons');
  await openPage(page, 'home');
  await expect.poll(() => page.evaluate(() => document.documentElement.dir)).toBe('rtl');
  const transformOf = (sel: string) =>
    page.locator(sel).first().evaluate((el) => getComputedStyle(el).transform);
  const back = page.getByRole('button', { name: 'Go back' }).locator('svg');
  await expect(back).toHaveClass(/\bflip\b/);
  expect(await back.evaluate((el) => getComputedStyle(el).transform)).toBe('matrix(-1, 0, 0, 1, 0, 0)');
  // No ancestor of the back glyph mirrors it again.
  const ancestorFlips = await back.evaluate((el) => {
    let n = el.parentElement;
    let flips = 0;
    while (n) {
      const t = getComputedStyle(n).transform;
      if (t.startsWith('matrix(-1')) flips++;
      n = n.parentElement;
    }
    return flips;
  });
  expect(ancestorFlips).toBe(0);
  // A non-directional glyph (the navigator toggle) is not mirrored.
  expect(await transformOf('button[aria-label="Open navigator"] svg')).toBe('none');
});
