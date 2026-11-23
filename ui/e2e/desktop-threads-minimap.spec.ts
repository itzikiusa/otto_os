import { test, expect } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { longThread, mockThread } from './threadsReadingFixture';

// ─────────────────────────────────────────────────────────────────────────────
// A long thread's minimap (RuntimeMinimap.svelte): one tick per message you
// sent at the reading column's edge — a failed turn and an approval marked —
// hover shows the message, a click jumps, the current one is lit, ⌥↑ / ⌥↓
// step; a short thread has none.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });
const SHOTS = process.env.OTTO_E2E_SHOTS ?? '';
let workspaceId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base, 'Minimap WS');
  await ctx.dispose();
});

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
});

test('a long thread has a minimap: marks, hover label, click jumps, the current turn is lit, ⌥↑/⌥↓ step', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const { items, turns } = longThread(10);
  await mockThread(page, workspaceId, 'mm-long', items, turns);
  await page.goto('/#/threads/mm-long');
  const thread = page.getByTestId('runtime-thread-mm-long');
  await expect(thread.getByText('Answer 10.')).toBeVisible({ timeout: 30_000 });
  const rail = thread.getByTestId('runtime-minimap');
  const ticks = rail.getByTestId('runtime-minimap-tick');
  await expect(ticks).toHaveCount(10);
  await expect(ticks.nth(2)).toHaveClass(/failed/);
  await expect(ticks.nth(4)).toHaveClass(/approval/);
  await expectFullyInViewport(page, rail, 'minimap');
  // It sits outside the reading column, not over the text.
  const [col, r] = [(await thread.locator('.rt-col').boundingBox())!, (await rail.boundingBox())!];
  expect(r.x).toBeGreaterThanOrEqual(col.x + col.width - 24);
  // Following the bottom: the last turn is lit.
  await expect(ticks.nth(9)).toHaveAttribute('aria-current', 'location');
  await ticks.nth(1).hover();
  await expect(rail.getByTestId('runtime-minimap-label')).toHaveText('Question number 2: what about part 2?');
  await ticks.nth(1).click();
  const u2 = thread.locator('[data-rt-msg="u2"]');
  await expect(u2).toBeInViewport();
  await expect(ticks.nth(1)).toHaveAttribute('aria-current', 'location');
  // ⌥↓ / ⌥↑ from the rail.
  await ticks.nth(1).focus();
  await page.keyboard.press('Alt+ArrowDown');
  await expect(thread.locator('[data-rt-msg="u3"]')).toBeInViewport();
  await expect(ticks.nth(2)).toHaveAttribute('aria-current', 'location');
  await page.keyboard.press('Alt+ArrowUp');
  await expect(ticks.nth(1)).toHaveAttribute('aria-current', 'location');
  await expectNoHorizontalOverflow(page);
  if (SHOTS) {
    mkdirSync(SHOTS, { recursive: true });
    await ticks.nth(4).hover();
    for (const s of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme: s });
      await page.screenshot({ path: `${SHOTS}/minimap-desktop-${s}.png`, animations: 'disabled' });
    }
    await page.emulateMedia({ colorScheme: 'light' });
  }
});

test('a short thread has no minimap', async ({ page }) => {
  const { items, turns } = longThread(4);
  await mockThread(page, workspaceId, 'mm-short', items, turns);
  await page.goto('/#/threads/mm-short');
  const thread = page.getByTestId('runtime-thread-mm-short');
  await expect(thread.getByText('Answer 4.')).toBeVisible({ timeout: 30_000 });
  await expect(thread.getByTestId('runtime-minimap')).toHaveCount(0);
});
