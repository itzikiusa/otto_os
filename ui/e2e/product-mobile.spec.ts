import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// Durable coverage for the Product (story analysis) page on small screens.
//
// Jira/Confluence is NOT connected in this environment, so live story DATA does
// not load — these assertions deliberately exercise only LAYOUT, fit, scroll,
// and navigation of the page chrome + empty/placeholder states:
//   • content fits the viewport width (no horizontal overflow);
//   • on a phone (≤640px) the page is a two-section accordion — a story-list
//     panel and a per-story content panel — each with a tappable header that
//     expands it, and the expanded panel is independently scrollable;
//   • the Stories|Learnings toggle and the story list stay usable;
//   • on wider screens the original side-by-side layout is preserved.

const PHONE = new Set(['iphone-portrait', 'iphone-se']);

let workspaceId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

async function openProduct(page: Page): Promise<void> {
  await page.goto('/#/product');
  await expect(page.locator('.product-page')).toBeVisible({ timeout: 30_000 });
  // A Stories|Learnings toggle is always present in some copy.
  await expect(
    page.locator('.product-header-row1 .vt:visible, .m-view-toggle .vt:visible').first(),
  ).toBeVisible({ timeout: 20_000 });
}

/** Visible width of an element, or 0 if absent/hidden. */
async function visibleWidth(page: Page, selector: string): Promise<number> {
  const box = await page.locator(selector).first().boundingBox();
  return box?.width ?? 0;
}

test('product: content fits the viewport width — no horizontal overflow', async ({ page }) => {
  await openProduct(page);
  await expectNoHorizontalOverflow(page);

  // The product page itself must not exceed the viewport width.
  const { vw, pageW } = await page.evaluate(() => {
    const el = document.querySelector('.product-page') as HTMLElement;
    return {
      vw: document.documentElement.clientWidth,
      pageW: Math.round(el.getBoundingClientRect().width),
    };
  });
  expect(pageW, 'product page width must fit the viewport').toBeLessThanOrEqual(vw + 2);
});

test('product: Stories|Learnings toggle + story list are usable', async ({ page }) => {
  await openProduct(page);

  // The story-list panel renders (empty-state copy with the import affordances).
  await expect(page.locator('.story-list')).toBeAttached();

  // Switch to Learnings via the visible toggle, then back to Stories.
  const learn = page.getByRole('tab', { name: 'Learnings' }).locator('visible=true').first();
  await learn.click();
  await expect(page.locator('.learn-nav')).toBeVisible();

  const stories = page.getByRole('tab', { name: 'Stories' }).locator('visible=true').first();
  await stories.click();
  await expect(page.locator('.story-list')).toBeVisible();
});

test('product (phone): accordion sections collapse/expand + scroll independently', async ({
  page,
}, testInfo) => {
  test.skip(!PHONE.has(testInfo.project.name), 'accordion is phone-only (≤640px)');
  await openProduct(page);

  const heads = page.locator('.m-acc-head');
  await expect(heads).toHaveCount(2);
  await expect(heads.nth(0)).toBeVisible();
  await expect(heads.nth(1)).toBeVisible();

  const pageEl = page.locator('.product-page');

  // List panel open by default → side has height, content collapsed to ~0.
  await heads.nth(0).click();
  await expect(pageEl).toHaveClass(/m-list-open/);
  expect(await visibleWidth(page, '.product-side')).toBeGreaterThan(0);
  const sideH1 = (await page.locator('.product-side').boundingBox())!.height;
  const mainH1 = (await page.locator('.product-main').boundingBox())?.height ?? 0;
  expect(sideH1, 'open list panel should have real height').toBeGreaterThan(80);
  expect(mainH1, 'collapsed content panel should be ~0 tall').toBeLessThan(8);

  // The open list panel is its own scroll container (overflow-y: auto).
  const sideOverflowY = await page
    .locator('.product-side')
    .evaluate((el) => getComputedStyle(el).overflowY);
  expect(sideOverflowY).toBe('auto');

  // Tap the content header → content panel expands, list collapses.
  await heads.nth(1).click();
  await expect(pageEl).toHaveClass(/m-content-open/);
  const sideH2 = (await page.locator('.product-side').boundingBox())?.height ?? 0;
  const mainH2 = (await page.locator('.product-main').boundingBox())!.height;
  expect(mainH2, 'open content panel should have real height').toBeGreaterThan(80);
  expect(sideH2, 'collapsed list panel should be ~0 tall').toBeLessThan(8);

  // The content panel's body is the scroll container.
  const bodyOverflowY = await page
    .locator('.product-body')
    .evaluate((el) => getComputedStyle(el).overflowY);
  expect(bodyOverflowY).toBe('auto');

  // Re-open the list — confirm it toggles back.
  await heads.nth(0).click();
  await expect(pageEl).toHaveClass(/m-list-open/);
});

test('product (wide): keeps the side-by-side layout, no accordion', async ({ page }, testInfo) => {
  test.skip(PHONE.has(testInfo.project.name), 'wide-layout assertion');
  await openProduct(page);

  // Accordion headers must not be shown above the phone breakpoint.
  await expect(page.locator('.m-acc-head').first()).toBeHidden();

  // Sidebar and main sit side by side (sidebar to the left of main).
  const side = (await page.locator('.product-side').boundingBox())!;
  const main = (await page.locator('.product-main').boundingBox())!;
  expect(side.x + side.width, 'sidebar should sit left of the main column').toBeLessThanOrEqual(
    main.x + 2,
  );
});
