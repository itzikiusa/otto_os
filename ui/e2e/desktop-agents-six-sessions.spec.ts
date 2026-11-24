import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Regression net for the Agents redesign (the owner's daily driver): six
// sessions started, tiled from the inbox, switched tabs ⇄ tiled ⇄ free layout,
// and all of them found again after a reload. The keyboard shortcuts (⌃1–⌃9,
// ⌘T) are unchanged. (Pop-out windows are Tauri-only; a daemon restart is
// covered by the daemon's own session re-adoption tests, not this shared harness.)

let ctx: APIRequestContext;
let base = '';
let wsId = '';
const TITLES = ['Maldini', 'Nesta', 'Pirlo', 'Kaka', 'Seedorf', 'Gattuso'];
const ids: Record<string, string> = {};

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  ({ ctx, base } = await apiCtx());
  wsId = await seedWorkspace(ctx, base);
  for (const title of TITLES) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
      data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta: { origin: 'e2e' } },
    });
    if (!r.ok()) throw new Error(`seed ${title} → ${r.status()} ${await r.text()}`);
    ids[title] = (await r.json()).id as string;
  }
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('e2e_seeded')) return;
    sessionStorage.setItem('e2e_seeded', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
    localStorage.setItem('otto_agents_fold_nav', '1');
  }, wsId);
});

test.afterEach(async () => {
  await ctx?.dispose();
});

const tiles = (page: Page) => page.locator('[data-tile-id]');

test('six sessions: tile, tabs ⇄ tiled ⇄ free layout, reload, shortcuts', async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#/agents');
  await expect(page.locator(`.ai-row[data-workspace="${wsId}"]`)).toHaveCount(6, { timeout: 30_000 });

  // Tile all six from the inbox's select mode.
  await page.getByTestId('agents-select-toggle').click();
  for (const t of TITLES) await page.getByLabel(`Select ${t}`).check();
  await page.getByTestId('agents-tile-selected').click();
  await expect(tiles(page)).toHaveCount(6, { timeout: 30_000 });

  // Tabs: a strip of six; ⌃3 jumps to the third tab (⌃1–⌃9, unchanged).
  await page.locator('button[aria-label="Tabbed view"]').click();
  await expect(page.locator('.tab')).toHaveCount(6, { timeout: 15_000 });
  await page.locator('.tab').first().locator('.tab-main').click();
  await page.keyboard.press('Control+3');
  await expect(page.locator('.tab').nth(2)).toHaveClass(/active/);

  // Back to tiled, then the free layout (split tree of the tiles).
  await page.locator('button[aria-label="Tiled view"]').click();
  await expect(tiles(page)).toHaveCount(6, { timeout: 15_000 });
  await page.getByRole('button', { name: /Free layout/ }).click();
  await expect(page.locator('[data-pane-key]')).toHaveCount(6, { timeout: 15_000 });

  // A reload finds every session and the layout again.
  await page.reload();
  await expect(page.locator(`.ai-row[data-workspace="${wsId}"]`)).toHaveCount(6, { timeout: 30_000 });
  await expect(page.locator('[data-pane-key]')).toHaveCount(6, { timeout: 30_000 });
  for (const t of TITLES) await expect(page.locator(`[data-pane-key][data-session="${ids[t]}"]`)).toHaveCount(1);

  // ⌘T still opens New session.
  await page.keyboard.press('Meta+t');
  await expect(page.getByRole('dialog', { name: 'New session' })).toBeVisible({ timeout: 10_000 });
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog', { name: 'New session' })).toHaveCount(0);
});
