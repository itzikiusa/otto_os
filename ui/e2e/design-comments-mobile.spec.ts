import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// Design Hall review comments on a phone (iPhone / Android projects; portrait
// and landscape), against the REAL /design/artifacts/{id}/comments routes:
// Comment mode drops a pin by TAP, the docked composer fits the screen and
// posts, and the Comments tab opens in the Details sheet (a pin tap opens it)
// with the thread, Reply and Resolve reachable — no horizontal page scroll.

const V1 = '/api/v1';
let wsId = '';
let stamp = '';

test.describe.configure({ timeout: 150_000 });

test.beforeAll(async () => {
  stamp = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, wsId);
});

async function design(title: string): Promise<string> {
  const { ctx, base } = await apiCtx();
  const r = await ctx.post(`${base}${V1}/design/artifacts`, {
    data: { workspace_id: wsId, format: 'html', studio: 'frames', title, content: '<!doctype html><h1 style="margin:40px">Rewards</h1>' },
  });
  expect(r.ok(), `create → ${r.status()} ${await r.text()}`).toBeTruthy();
  const id = (await r.json()).artifact.id as string;
  await ctx.dispose();
  return id;
}

async function open(page: Page, id: string): Promise<void> {
  await page.goto(`/#/design/a/${id}`);
  await expect(page.getByTestId('design-more')).toBeVisible({ timeout: 90_000 });
  await expect(page.getByTestId('design-pin-layer')).toBeAttached({ timeout: 30_000 });
}

test('tap to pin a comment, then read and resolve it in the Details sheet', async ({ page }) => {
  const id = await design(`Phone pins ${stamp}`);
  await open(page, id);
  await expectNoHorizontalOverflow(page);

  const mode = page.getByTestId('design-comment-mode');
  await expectFullyInViewport(page, mode);
  await mode.tap();
  const layer = (await page.getByTestId('design-pin-layer').boundingBox())!;
  await page.getByTestId('design-pin-drop').tap({ position: { x: layer.width * 0.5, y: Math.min(layer.height * 0.3, 120) } });
  const composer = page.getByTestId('design-pin-composer');
  await expect(composer).toBeVisible();
  await expectFullyInViewport(page, composer);
  await composer.getByRole('textbox', { name: 'Comment text' }).fill('Headline too close to the edge');
  await composer.getByRole('button', { name: 'Post', exact: true }).tap();
  const pin = page.getByTestId('design-pin');
  await expect(pin).toHaveCount(1);
  await expectNoHorizontalOverflow(page);

  // Tapping the pin opens the Comments tab in the Details sheet.
  await mode.tap(); // leave comment mode
  await pin.tap();
  await expect(page.getByTestId('design-tab-comments')).toHaveAttribute('aria-selected', 'true');
  const thread = page.getByTestId('design-comment');
  await expect(thread).toContainText('Headline too close to the edge');
  await expectNoHorizontalOverflow(page);
  await expectFullyInViewport(page, thread.getByTestId('design-comment-resolve'));
  await thread.getByTestId('design-comment-resolve').tap();
  await expect(page.getByTestId('design-comments-resolved')).toHaveText(/Resolved \(1\)/);
  await page.getByRole('button', { name: 'Back to design', exact: true }).tap();
  await expect(page.getByTestId('design-stage')).toBeVisible();
  await expect(page.getByTestId('design-pin')).toHaveCount(0);
});

test('an empty design explains how to comment, in the sheet', async ({ page }) => {
  const id = await design(`Phone empty ${stamp}`);
  await open(page, id);
  await page.getByRole('button', { name: 'Show design details', exact: true }).tap();
  await page.getByTestId('design-tab-comments').tap();
  await expect(page.getByTestId('design-comments')).toContainText('No comments yet');
  await expect(page.getByTestId('design-comments')).toContainText('Press C or use Comment to pin one on the design.');
  await expectNoHorizontalOverflow(page);
});
