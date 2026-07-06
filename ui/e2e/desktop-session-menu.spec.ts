import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Session-card ⋯ menu with MANY open sessions (desktop-browser only).
//
// Regression: the ⋯ dropdown was a hand-rolled `position:absolute; top:22px`
// popup that always opened downward with no viewport clamp and no max-height.
// On a bottom-row tile (many sessions open → the tiled grid pushes headers
// toward the window's bottom edge) the tail items — Pin / Archive / Delete —
// rendered below the viewport and were unreachable. The menu now routes
// through the global clamped ctxMenu store and must stay fully inside the
// viewport with every entry reachable.
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let wsId = '';
const TITLES = ['Menu-A', 'Menu-B', 'Menu-C', 'Menu-D'];

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  wsId = await seedWorkspace(ctx, base);
  for (const title of TITLES) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
      data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta: { origin: 'e2e' } },
    });
    if (!r.ok()) throw new Error(`seed ${title} → ${r.status()} ${await r.text()}`);
  }
});

test.afterAll(async () => {
  await ctx?.dispose();
});

test.beforeEach(async ({ page: _page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
});

test('session ⋯ menu stays inside the viewport on a bottom-row tile', async ({ page }) => {
  // Short window: the 2×2 tiled grid puts the bottom row's header low enough
  // that the (~12-item) menu cannot fit below it without clamping.
  await page.setViewportSize({ width: 1200, height: 520 });
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
  await page.goto('/#/agents');
  // The page opens in tabbed view with no open tabs — switch to tiled so every
  // seeded session renders as a grid tile.
  await page.locator('button[aria-label="Tiled view"]').click();
  await expect(page.locator('.pane')).toHaveCount(TITLES.length, { timeout: 30_000 });

  // Pick the tile whose header sits lowest in the viewport.
  const heads = page.locator('.pane-head');
  const n = await heads.count();
  let lowIdx = 0;
  let lowY = -1;
  for (let i = 0; i < n; i++) {
    const b = await heads.nth(i).boundingBox();
    if (b && b.y > lowY) {
      lowY = b.y;
      lowIdx = i;
    }
  }
  await heads.nth(lowIdx).locator('button[title="More…"]').click();

  const menu = page.locator('.ctx-menu');
  await expect(menu).toBeVisible();
  // Give the post-render clamp (rAF) a beat to land before measuring.
  await page.waitForTimeout(100);

  await expectFullyInViewport(page, menu, 'session ⋯ menu');

  // The first action is visible…
  await expect(menu.getByRole('menuitem', { name: 'Rename…' })).toBeVisible();
  // …and the LAST action (Delete) is reachable (menu scrolls internally).
  const del = menu.getByRole('menuitem', { name: 'Delete' });
  await del.scrollIntoViewIfNeeded();
  await expect(del).toBeVisible();
  await page.keyboard.press('Escape');
});
