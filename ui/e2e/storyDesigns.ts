import { expect, type Page } from '@playwright/test';

// Design Hall is the one home for designs (docs/features/design-hall.md §4.1).
// The story Design tab is the Hall's grid (StoryDesigns); the old DesignArena
// opens only from its ⋯ → "Open the classic arena", and `#/canvas` forwards
// into the Hall. Specs that exercise the OLD editors go through these helpers.

/** Why a spec that drives the old Canvas page editor is skipped: the page is
 *  unmounted (kept on disk; the `canvas` loader in shell/pages.svelte.ts points
 *  at CanvasRedirect.svelte). Point the loader back at CanvasPage.svelte and
 *  these run again. Canvas boards are edited as Design Hall whiteboards. */
export const CANVAS_PAGE_RETIRED =
  'Canvas page folded into Design Hall: #/canvas forwards to the Hall, so the unmounted CanvasPage editor is not reachable';

/** On a story's Design tab (StoryDesigns), open ⋯ → "Open the classic arena"
 *  and wait for the arena (`.mockups-tab` is its alias class). */
export async function openClassicArena(page: Page): Promise<void> {
  const designs = page.getByTestId('story-designs');
  await expect(designs).toBeVisible({ timeout: 15_000 });
  await page.getByTestId('story-designs-more').click();
  await page.getByRole('menuitem', { name: /Open the classic arena/ }).click();
  await expect(page.getByTestId('story-designs-classic')).toBeVisible({ timeout: 10_000 });
  await expect(page.locator('.mockups-tab')).toBeVisible({ timeout: 15_000 });
}
