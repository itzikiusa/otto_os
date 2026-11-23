import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';
import { ITEMS, mockThread, TURNS } from './threadsReadingFixture';

// The floating thread details card beside the 760 px reading column
// (ThreadDetailsCard + RuntimeThread's .rt-col): at 1280, 1440, 1600 and 1920
// with the thread list open and closed, the card floats exactly when one
// reading column fits beside it, and then nothing of the conversation — the
// messages, the end-of-turn changes card, the composer card — runs under it.
// THRC_SHOTS=<dir> keeps the integrated page at 1600 and on a phone.

test.use({ serviceWorkers: 'block' });
let workspaceId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base, 'Details layout WS');
  await ctx.dispose();
});

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
});

type Box = { x: number; y: number; width: number; height: number };
const overlaps = (a: Box, b: Box): boolean => a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;

async function shots(page: Page, name: string): Promise<void> {
  const out = process.env.THRC_SHOTS;
  if (!out) return;
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: `${out}/${name}-${scheme}.png`, animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

test('the details card never covers the reading column, at any width, list open or closed', async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1600, height: 1000 });
  await mockThread(page, workspaceId, 'lay-1', ITEMS, TURNS);
  await page.goto('/#/threads/lay-1');
  const thread = page.getByTestId('runtime-thread-lay-1');
  await expect(thread.getByText('Offline edits now survive')).toBeVisible({ timeout: 30_000 });
  await shots(page, 'integrated-1600');
  const toggle = page.getByTestId('threads-toggle-sidebar');
  for (const width of [1280, 1440, 1600, 1920]) {
    for (const listOpen of [true, false]) {
      await page.setViewportSize({ width, height: 1000 });
      if ((await toggle.getAttribute('aria-pressed')) !== String(listOpen)) await toggle.click();
      await expect(toggle).toHaveAttribute('aria-pressed', String(listOpen));
      const centre = (await page.locator('.th-center-col').boundingBox())!;
      const shouldFloat = centre.width >= 760 + 48 + 328;
      const card = page.getByTestId('thread-details-card');
      await expect(card, `${width} list ${listOpen ? 'open' : 'closed'}: floats`).toHaveCount(shouldFloat ? 1 : 0);
      if (!shouldFloat) continue;
      const c = (await card.boundingBox())!;
      const parts = [
        thread.locator('.rt-composer'),
        thread.getByTestId('runtime-turn-changes'),
        thread.getByText('Offline edits now survive'),
        thread.locator('[data-testid^="runtime-fold-"]').first(),
      ];
      for (const el of parts) {
        const b = (await el.boundingBox())!;
        expect(overlaps(b, c), `${width} list ${listOpen ? 'open' : 'closed'}: the conversation runs under the card`).toBe(false);
      }
      // The column still gets its full measure beside the card.
      expect((await thread.locator('.rt-col').boundingBox())!.width).toBeGreaterThanOrEqual(760);
      await expectNoHorizontalOverflow(page);
    }
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(thread.getByText('Offline edits now survive')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await shots(page, 'integrated-phone');
});
