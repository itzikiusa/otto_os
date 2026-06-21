import { test } from '@playwright/test';
import {
  PAGES,
  openPage,
  expectNoHorizontalOverflow,
  expectContentHasHeight,
  expectAccessible,
} from './helpers';

// Baseline coverage for EVERY top-level page, run across all device profiles
// (iPhone portrait/landscape, iPad portrait/landscape, iPhone SE) via the
// Playwright projects in playwright.config.ts.
//
// Each page must, on every device:
//   1. render the shell + a content pane that actually has height
//      (catches collapsed-pane bugs: blank terminal, missing DB results)
//   2. not overflow the viewport horizontally
//      (catches clipped-content bugs: Git graph / PR diff running off-screen)
//   3. have no `critical` accessibility violations
//
// Deeper per-page interaction specs (drill-down + tabs) live in their own files.

for (const id of PAGES) {
  test.describe(`page: ${id}`, () => {
    test('renders with a sized content pane', async ({ page }) => {
      await openPage(page, id);
      await expectContentHasHeight(page);
    });

    test('does not overflow horizontally', async ({ page }) => {
      await openPage(page, id);
      await expectNoHorizontalOverflow(page);
    });

    test('has no critical accessibility violations', async ({ page }) => {
      await openPage(page, id);
      await expectAccessible(page);
    });
  });
}
