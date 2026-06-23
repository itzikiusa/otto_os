import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ── E2E: Canvas Studio ─────────────────────────────────────────────────────────
//
// Drives the full Canvas flow against the isolated OFFLINE test daemon:
//   create-from-template → editor renders → Ask AI (stubbed agent) inserts a node
//   → Present mode steps + exits → the scene persists across a reload.
//
// The throwaway daemon runs with OTTO_E2E=1, so `/canvas/assist/*` returns the
// deterministic stub (a mermaid sequence block) instead of spawning a real CLI —
// the assist flow is reproducible and network-free.

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

test('canvas: template → editor → Ask AI inserts a node → Present → persists', async ({
  page,
}) => {
  test.setTimeout(90_000);
  await openCanvas(page);

  // Empty-scene hero offers templates; pick the Sequence diagram template.
  const tpl = page.locator('.tpl', { hasText: 'Sequence diagram' }).first();
  await expect(tpl).toBeVisible({ timeout: 15_000 });
  await tpl.click();

  // The editor mounts and renders the template's mermaid node.
  const flow = page.locator('.svelte-flow');
  await expect(flow).toBeVisible({ timeout: 20_000 });
  const nodes = page.locator('.svelte-flow__node');
  await expect.poll(() => nodes.count(), { timeout: 15_000 }).toBeGreaterThanOrEqual(1);
  const initialCount = await nodes.count();

  // Ask AI → the floating pill → type a prompt → Draw. The stubbed agent returns
  // a mermaid block, which `insertAssist` adds as a new node.
  await page.getByRole('button', { name: 'Ask AI' }).click();
  const pill = page.locator('.pill input');
  await expect(pill).toBeVisible({ timeout: 8_000 });
  await pill.fill('service A calls B; B does several things');
  await page.getByRole('button', { name: 'Draw' }).click();

  // A node was inserted (count grew) — assert on the node count, not the toast.
  await expect
    .poll(() => nodes.count(), { timeout: 20_000 })
    .toBeGreaterThan(initialCount);

  // Present mode: opens a full-screen overlay; → advances, Esc exits.
  await page.locator('.pill .close').click().catch(() => {});
  await page.getByRole('button', { name: 'Present' }).click();
  const present = page.locator('.present');
  await expect(present).toBeVisible({ timeout: 10_000 });
  await page.keyboard.press('ArrowRight');
  await page.keyboard.press('Escape');
  await expect(present).toBeHidden({ timeout: 8_000 });

  // Persistence: the scene autosaves; reload and confirm it's in the list.
  await page.waitForTimeout(1200); // let the debounced autosave flush
  await page.reload();
  await expect(page.locator('.canvas-page')).toBeVisible({ timeout: 30_000 });
  await expect(
    page.locator('.scene-list', { hasText: 'Sequence diagram' }).first(),
  ).toBeVisible({ timeout: 20_000 });
});

test.describe('canvas on a phone', () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test('canvas: phone shows the read-only editor banner', async ({ page }) => {
    test.setTimeout(60_000);
    await openCanvas(page);
    // Create a blank scene from the hero, then assert the read-only banner.
    await page.getByRole('button', { name: /blank canvas/i }).click();
    await expect(page.locator('.ro-banner')).toBeVisible({ timeout: 20_000 });
    // Present is still available on a phone.
    await expect(page.getByRole('button', { name: 'Present' })).toBeVisible();
  });
});
