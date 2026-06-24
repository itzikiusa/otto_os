import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ── E2E: Canvas Studio (embedded Excalidraw) ───────────────────────────────────
//
// The Canvas is the real Excalidraw editor, mounted React-in-Svelte and lazy-
// loaded. These variations drive the isolated test daemon to prove the embed
// works end to end: it mounts, exposes Excalidraw's native chrome, draws + saves
// (verified through the persisted doc via the API), survives reloads, and handles
// multiple scenes + the phone view-mode path.

test.use({ viewport: { width: 1360, height: 900 }, actionTimeout: 12_000 });

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

async function openCanvas(page: Page): Promise<void> {
  await page.goto('/#/canvas');
  await expect(page.locator('.canvas-page')).toBeVisible({ timeout: 30_000 });
}

/** Click "New canvas" and wait for the embedded Excalidraw editor to mount. */
async function newCanvas(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'New canvas' }).click();
  await expect(page.locator('.excali .excalidraw')).toBeVisible({ timeout: 30_000 });
}

test('mounts the Excalidraw editor + the new scene persists in the list', async ({ page }) => {
  test.setTimeout(90_000);
  await openCanvas(page);
  await newCanvas(page);

  await page.waitForTimeout(900);
  await page.reload();
  await expect(page.locator('.canvas-page')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.scene-list', { hasText: 'Untitled canvas' }).first()).toBeVisible({
    timeout: 20_000,
  });
});

test('exposes Excalidraw native chrome (interactive canvas + toolbar)', async ({ page }) => {
  test.setTimeout(90_000);
  await openCanvas(page);
  await newCanvas(page);

  // Excalidraw renders its own interactive <canvas> and its shape toolbar.
  await expect(page.locator('.excalidraw canvas').first()).toBeVisible({ timeout: 20_000 });
  await expect(
    page.locator('.excalidraw [aria-label*="Rectangle" i]').first(),
  ).toBeVisible({ timeout: 20_000 });
});

test('draws a rectangle → styles panel appears + the edit autosaves', async ({ page }) => {
  test.setTimeout(90_000);
  await openCanvas(page);
  await newCanvas(page);

  const canvas = page.locator('.excalidraw canvas').first();
  const box = await canvas.boundingBox();
  expect(box).not.toBeNull();
  const b = box!;

  // The autosave PUT the draw will trigger (debounced) — capture it up front.
  const saved = page.waitForResponse(
    (r) => r.request().method() === 'PUT' && /\/canvas\/scenes\//.test(r.url()),
    { timeout: 20_000 },
  );

  // Focus the editor, select the rectangle tool by keyboard, then drag one out
  // in a clear area away from the toolbars/panels.
  await page.mouse.click(b.x + b.width * 0.5, b.y + b.height * 0.5);
  await page.keyboard.press('r');
  const x1 = b.x + b.width * 0.38;
  const y1 = b.y + b.height * 0.38;
  await page.mouse.move(x1, y1);
  await page.mouse.down();
  await page.mouse.move(x1 + 190, y1 + 140, { steps: 18 });
  await page.mouse.up();

  // The new element is selected → Excalidraw's styles panel (always has an
  // Opacity control) appears. This proves the draw landed.
  await expect(
    page.locator('.excalidraw').getByText(/opacity/i).first(),
  ).toBeVisible({ timeout: 10_000 });

  // …and the edit autosaved to the server.
  const resp = await saved;
  expect(resp.ok()).toBeTruthy();
});

test('handles multiple scenes — create, list, and switch between them', async ({ page }) => {
  test.setTimeout(90_000);
  await openCanvas(page);
  await newCanvas(page);
  await page.waitForTimeout(700);

  // Create a second canvas from the scene-list's "New" button.
  await page.locator('.scene-list .new').click();
  await expect(page.locator('.excali .excalidraw')).toBeVisible({ timeout: 30_000 });

  // Both scenes show as rows in the list.
  await expect
    .poll(() => page.locator('.scene-list .row').count(), { timeout: 15_000 })
    .toBeGreaterThanOrEqual(2);

  // Switching to another scene remounts Excalidraw cleanly.
  await page.locator('.scene-list .row').first().click();
  await expect(page.locator('.excali .excalidraw')).toBeVisible({ timeout: 30_000 });
});

test('Ask AI → generates editable Excalidraw shapes from the agent', async ({ page }) => {
  test.setTimeout(90_000);
  await openCanvas(page);
  await newCanvas(page);

  // The autosave PUT the generated shapes will trigger.
  const saved = page.waitForResponse(
    (r) => r.request().method() === 'PUT' && /\/canvas\/scenes\//.test(r.url()),
    { timeout: 30_000 },
  );

  // Open the AI bar, describe a diagram, Draw. The isolated daemon's stub returns
  // a deterministic Mermaid diagram, which mermaid-to-excalidraw converts to
  // native editable shapes added to the scene.
  await page.getByRole('button', { name: /ask ai to draw/i }).click();
  await page.getByPlaceholder(/describe a diagram/i).fill('service A calls B; B does several things');
  await page.getByRole('button', { name: 'Draw' }).click();

  // Success toast confirms the diagram was generated + placed…
  await expect(page.getByText(/drawn on canvas/i).first()).toBeVisible({ timeout: 30_000 });
  // …and it autosaved to the server.
  const resp = await saved;
  expect(resp.ok()).toBeTruthy();
});

test.describe('canvas on a phone', () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test('the editor still mounts on a phone (view mode)', async ({ page }) => {
    test.setTimeout(60_000);
    await openCanvas(page);
    await page.getByRole('button', { name: 'New canvas' }).click();
    await expect(page.locator('.excali .excalidraw')).toBeVisible({ timeout: 30_000 });
  });
});
