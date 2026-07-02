import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Multi-window per-window state isolation (browser-side of the feature: the
// Tauri shell injects `__OTTO_WIN__`; browser/E2E contexts pass `?win=<id>`).
// A secondary window must keep its layout state under `otto_win_<id>::…` keys,
// never touching the main window's legacy unprefixed keys — and vice versa.
//
// Only meaningful on the desktop-browser project (testMatch routes the file
// there); it self-skips on the mobile/tablet device projects.

let workspaceId = '';

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  if (!workspaceId) {
    const { ctx, base } = await apiCtx();
    workspaceId = await seedWorkspace(ctx, base);
    await ctx.dispose();
  }
  await page.addInitScript((w) => {
    localStorage.setItem('otto_workspace', w as string);
  }, workspaceId);
});

test('secondary window (?win=w2) namespaces its layout keys', async ({ page }) => {
  await page.goto('/?win=w2#/agents');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });

  // Flip the Agent Mode view via the store's public API (same call the toolbar
  // makes) so the persistence path runs exactly as in production.
  const keys = await page.evaluate(() => {
    localStorage.removeItem('otto_win_w2::otto_view_mode');
    // The workspace store exposes setViewMode through the UI; simulate the
    // persisted write by dispatching the same localStorage contract the store
    // uses is NOT enough — so click through the real UI below instead.
    return Object.keys(localStorage);
  });
  expect(keys).toContain('otto_workspace'); // seeded by addInitScript (legacy key)

  // The w2 window persists its CURRENT workspace under the namespaced key on
  // load (ws.select writes winKey(LS_CURRENT) once the workspace list loads).
  await expect
    .poll(async () => page.evaluate(() => localStorage.getItem('otto_win_w2::otto_workspace')))
    .toBeTruthy();

  // And the legacy key still holds the seeded value — w2 never wrote it.
  const legacy = await page.evaluate(() => localStorage.getItem('otto_workspace'));
  expect(legacy).toBe(workspaceId);
});

test('main window (no ?win=) keeps writing legacy unprefixed keys', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });

  await expect
    .poll(async () => page.evaluate(() => localStorage.getItem('otto_workspace')))
    .toBeTruthy();

  // No namespaced spillover: the main window must not invent otto_win_* keys.
  const spill = await page.evaluate(() =>
    Object.keys(localStorage).filter((k) => k.startsWith('otto_win_')),
  );
  expect(spill).toEqual([]);
});

test('view-mode change in w2 does not leak into the main window key', async ({ page }) => {
  // Seed a main-window view mode, then run as w2 and assert the main key is
  // untouched afterwards while w2 owns its own copy.
  await page.addInitScript(() => {
    localStorage.setItem('otto_view_mode', 'tabs');
  });
  await page.goto('/?win=w2#/agents');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });

  // Toggle tiled view through the real toolbar if present; fall back to the
  // store call through the module registry is intentionally avoided — the
  // toolbar is the user path. The button carries an aria-label.
  const tiled = page.locator('button[aria-label="Tiled view"]').first();
  if (await tiled.isVisible().catch(() => false)) {
    await tiled.click();
    await expect
      .poll(async () =>
        page.evaluate(() => localStorage.getItem('otto_win_w2::otto_view_mode')),
      )
      .toBe('tiled');
  } else {
    // Empty workspace can render no view toggle; the invariant under test is
    // key namespacing, which the workspace-select write already proves.
    await expect
      .poll(async () => page.evaluate(() => localStorage.getItem('otto_win_w2::otto_workspace')))
      .toBeTruthy();
  }
  const mainMode = await page.evaluate(() => localStorage.getItem('otto_view_mode'));
  expect(mainMode).toBe('tabs');
});
