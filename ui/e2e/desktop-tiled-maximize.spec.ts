import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// Tiled view — maximize ("zoom in on this session") and restore.
//
// The maximized tile is mounted straight into `.agents-body` (a plain block),
// NOT into the `.tiled-wrap` flex column the grid lives in. When the grid's
// container switched from `height: 100%` to `flex: 1` (the Free-layout bar
// commit) the single-tile branch silently lost its height: the pane header
// rendered and the terminal body collapsed to 0px — a black screen. This spec
// pins the maximized pane's BODY to the viewport, not just its header.
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let wsId = '';
const TITLES = ['Maldini', 'Baresi'];

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
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
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
  }, wsId);
  await page.goto('/#/agents');
  await expect(page.getByText(TITLES[0]).first()).toBeVisible({ timeout: 20_000 });
});

test.afterEach(async () => {
  await ctx?.dispose();
});

test('maximizing a tile fills the body with a live terminal, and restore brings the grid back', async ({ page }) => {
  await page.locator('.nav-item.nested-item', { hasText: TITLES[0] }).first().click();
  await expect(page.locator('[data-pane-key]').first()).toBeVisible({ timeout: 15_000 });
  await page.locator('button[aria-label="Tiled view"]').click();
  await expect(page.locator('[data-tile-id]')).toHaveCount(2, { timeout: 20_000 });

  // Zoom in on the first tile.
  await page.locator('[data-tile-id]').first().locator('button[title="Zoom in on this session"]').click();
  await expect(page.locator('[data-tile-id]')).toHaveCount(0);
  const pane = page.locator('.tiled.single .pane');
  await expect(pane).toHaveCount(1);

  // The regression: header present, body 0px tall. The single pane must take
  // the agents body, and its terminal body most of that.
  const bodyH = await page.locator('.agents-body').evaluate((el) => el.getBoundingClientRect().height);
  expect(bodyH).toBeGreaterThan(400);
  await expect
    .poll(() => pane.evaluate((el) => el.getBoundingClientRect().height))
    .toBeGreaterThan(bodyH - 40);
  const termBody = pane.locator('.pane-body');
  await expect.poll(() => termBody.evaluate((el) => el.getBoundingClientRect().height)).toBeGreaterThan(bodyH * 0.7);
  await expect(pane.locator('.term-host')).toBeVisible();

  // Restore → the grid again, both tiles.
  await pane.locator('button[title="Restore tiled view"]').click();
  await expect(page.locator('[data-tile-id]')).toHaveCount(2, { timeout: 15_000 });
});
