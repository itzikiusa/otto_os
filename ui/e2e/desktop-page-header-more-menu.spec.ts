import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// PageHeader "⋯" overflow menu: anchored + keyboard-drivable (desktop-browser).
//
// Regression 1: a button-opened menu opened at the POINTER (or, for the ⋯
// button, at a hard-coded `right - 180px` guess), so it floated wherever the
// click landed instead of hanging off its trigger. It now opens directly under
// the button, end edges aligned (ctxMenu.showAt(…, { align: 'end' })).
//
// Regression 2: the menu only understood Esc. It now takes focus (first
// enabled row), walks with ↑/↓, activates with Enter, and Esc hands focus
// back to the ⋯ button.
// ─────────────────────────────────────────────────────────────────────────────

// Pages whose header carries enough collapsible actions to overflow once the
// window is narrowed; the first one that shows a "⋯" is used.
const CANDIDATES = ['product', 'swarm', 'workflows', 'vault', 'api'];
const WIDTHS = [1100, 960, 820, 700];

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, wsId);
});

/** Find a page + width where the header folds actions into "⋯". */
async function findMore(page: Page) {
  for (const id of CANDIDATES) {
    await page.setViewportSize({ width: 1280, height: 800 });
    await openPage(page, id);
    for (const width of WIDTHS) {
      await page.setViewportSize({ width, height: 800 });
      const more = page.locator('[data-testid="page-header"] .ph-more').first();
      try {
        await expect(more).toBeVisible({ timeout: 1_500 });
        return more;
      } catch {
        /* try narrower / the next page */
      }
    }
  }
  throw new Error('no page header folded its actions into ⋯ at any tested width');
}

test('⋯ menu opens under its button and is keyboard-drivable', async ({ page }) => {
  const more = await findMore(page);
  const menu = page.locator('.ctx-menu');

  await more.click();
  await expect(menu).toBeVisible();
  await page.waitForTimeout(100); // post-render measure/clamp (rAF)
  await expectFullyInViewport(page, menu, 'page header ⋯ menu');

  // Directly under the trigger, end edges aligned (unless clamped at the edge).
  const b = (await more.boundingBox())!;
  const m = (await menu.boundingBox())!;
  expect(Math.abs(m.y - (b.y + b.height + 4))).toBeLessThanOrEqual(2);
  const vw = page.viewportSize()!.width;
  const clampedAtEdge = m.x + m.width >= vw - 8 - 1;
  if (!clampedAtEdge) expect(Math.abs(m.x + m.width - (b.x + b.width))).toBeLessThanOrEqual(2);

  // Focus lands on the first enabled row; ↓ moves to the next.
  const rows = menu.locator('.ctx-item:not(:disabled)');
  await expect(rows.first()).toBeFocused();
  if ((await rows.count()) > 1) {
    await page.keyboard.press('ArrowDown');
    await expect(rows.nth(1)).toBeFocused();
    await page.keyboard.press('ArrowUp');
    await expect(rows.first()).toBeFocused();
  }

  // Esc closes and returns focus to the trigger…
  await page.keyboard.press('Escape');
  await expect(menu).toBeHidden();
  await expect(more).toBeFocused();

  // …Enter on the focused trigger re-opens it (anchored the same way)…
  await page.keyboard.press('Enter');
  await expect(menu).toBeVisible();
  await expect(rows.first()).toBeFocused();
  const m2 = (await menu.boundingBox())!;
  expect(Math.abs(m2.y - (b.y + b.height + 4))).toBeLessThanOrEqual(2);

  // …and Enter on a row activates it. The row either ran its action (menu
  // closed) or — a collapsed "New ▾"-style button — opened ITS menu, which
  // must stay open (not be closed by the row that opened it), anchored to the
  // still-visible ⋯ button, focused, and closable back to ⋯.
  const firstLabel = await rows.first().textContent();
  await page.keyboard.press('Enter');
  await expect
    .poll(async () => ((await menu.isVisible()) ? await rows.first().textContent() : '__closed__'))
    .not.toBe(firstLabel);
  if (await menu.isVisible()) {
    await page.waitForTimeout(100);
    await expectFullyInViewport(page, menu, 'cascaded menu');
    const m3 = (await menu.boundingBox())!;
    expect(Math.abs(m3.y - (b.y + b.height + 4))).toBeLessThanOrEqual(2);
    await expect(rows.first()).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(menu).toBeHidden();
    await expect(more).toBeFocused();
  }
});
