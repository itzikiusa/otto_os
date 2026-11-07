import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// Durable coverage for the Product (story analysis) page on small screens.
//
// Jira/Confluence is NOT connected in this environment, so live story DATA does
// not load — these assertions deliberately exercise only LAYOUT, fit, scroll,
// and navigation of the page chrome + empty/placeholder states:
//   • content fits the viewport width (no horizontal overflow);
//   • on a phone (≤640px) story selection opens its content and Back returns to the list;
//   • the Stories|Learnings toggle and the story list stay usable;
//   • on wider screens the original side-by-side layout is preserved.

const PHONE = new Set(['iphone-portrait', 'iphone-se', 'android-phone']);

let workspaceId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  // One blank draft so the story-list pane exists: a workspace with NO stories
  // hides the (empty) list pane on ≥641px and shows the page empty state.
  const r = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/product/drafts`, { data: {} });
  if (!r.ok()) throw new Error(`draft seed failed: ${r.status()} ${await r.text()}`);
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
    page.locator('.m-view-toggle > button:visible').first(),
  ).toBeVisible({ timeout: 20_000 });
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
  // Learnings is one full-width view now (no separate learnings nav pane).
  await expect(page.locator('.learnings-view')).toBeVisible();

  const stories = page.getByRole('tab', { name: 'Stories' }).locator('visible=true').first();
  await stories.click();
  await expect(page.locator('.story-list')).toBeVisible();
});

test('product (phone): story selection and Back use the available reading space', async ({ page }, info) => {
  test.skip(!PHONE.has(info.project.name), 'phone push navigation');
  await openProduct(page);
  await page.locator('.story-row').first().click();
  const main = page.locator('.product-main');
  await expect(page.locator('.product-page')).toHaveClass(/m-content-open/);
  await expect(page.getByRole('button', { name: 'Back to stories' })).toBeVisible();
  await expect(page.locator('.m-acc-head')).toHaveCount(0);
  expect((await main.boundingBox())!.height).toBeGreaterThan(200);
  await expect(page.locator('.story-details')).not.toHaveAttribute('open');
  await page.locator('.story-details > summary').click();
  await expect(page.locator('.story-details')).toHaveAttribute('open');
  await page.getByRole('button', { name: 'Back to stories' }).click();
  await expect(page.locator('.product-page')).toHaveClass(/m-list-open/);
  await expect(page.locator('.story-list')).toBeVisible();
  expect((await main.boundingBox())?.height ?? 0).toBeLessThan(8);
  await expectNoHorizontalOverflow(page);
});

test('product (wide): keeps the side-by-side layout, no accordion', async ({ page }, testInfo) => {
  test.skip(PHONE.has(testInfo.project.name), 'wide-layout assertion');
  await openProduct(page);

  // Push-navigation headers must not duplicate the shared toolbar.
  await expect(page.locator('.m-acc-head')).toHaveCount(0);

  // Sidebar and main sit side by side (sidebar to the left of main).
  const side = (await page.locator('.product-side').boundingBox())!;
  const main = (await page.locator('.product-main').boundingBox())!;
  expect(side.x + side.width, 'sidebar should sit left of the main column').toBeLessThanOrEqual(
    main.x + 2,
  );
});
