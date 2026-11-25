import { test, expect } from '@playwright/test';

// A new IDE editor is built from its current props, so the prop effects of the flush that built
// it have nothing to send. A prop that changes right after, in the same tick, still must reach
// the editor: here a breakpoint set while the editor mounts shows in its gutter.
test('a breakpoint set in the same tick as the editor mounts shows in the gutter', async ({ page }) => {
  await page.goto('/e2e/fixtures/ide-editor-same-tick.html');
  // The first editor loads the IDE extensions once (semantic colours appear when they are in).
  await expect(page.locator('[class*="cm-syn-"]').first()).toBeVisible({ timeout: 15_000 });
  expect(await page.evaluate(() => (window as unknown as { remountWithBreakpoint: () => boolean }).remountWithBreakpoint())).toBe(true);
  await expect(page.getByTestId('editor-for')).toHaveAttribute('data-path', '/w/b.go');
  await expect(page.locator('.cm-ide-bp-gutter .cm-ide-bp-marker')).toHaveCount(1);
});
