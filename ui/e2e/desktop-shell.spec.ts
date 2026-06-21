import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';

// Desktop BROWSER (≥1025px, non-Tauri) regression guard. The remote-desktop path
// must NOT apply the app's CSS `zoom` to the shell even when an app zoom is set
// (otto_zoom): CSS zoom stretches the WebGL terminal (oversized + clipped fit)
// and skews click hit-testing + popover/dropdown coordinates. Browsers zoom
// natively (crisp). Tauri uses native page-zoom. So under any otto_zoom the shell
// has no inline `zoom`, the terminal fits, and the sidebar toggle still hits.
//
// Only meaningful on the desktop-browser project (testMatch in the config routes
// the file there); it self-skips on the mobile/tablet device projects.

let workspaceId = '';
let sessionId = '';

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  if (!workspaceId) {
    const { ctx, base } = await apiCtx();
    workspaceId = await seedWorkspace(ctx, base);
    sessionId = await seedShellSession(ctx, base, workspaceId);
    await ctx.post(`${base}/api/v1/sessions/${sessionId}/input`, {
      data: { text: 'for i in $(seq 1 30); do echo "OTTO-LINE-$i"; done', submit: true },
    });
    await new Promise((r) => setTimeout(r, 1200));
    await ctx.dispose();
  }
  await page.addInitScript(
    (w) => {
      localStorage.setItem('otto_workspace', w as string);
      localStorage.setItem('otto_zoom', '2'); // the condition that used to break it
    },
    workspaceId,
  );
});

test('shell applies NO CSS zoom in a browser, even with otto_zoom set', async ({ page }) => {
  await page.goto(`/#/agents/${sessionId}`);
  await expect(page.locator('.term-host')).toBeVisible({ timeout: 30_000 });
  await page.waitForTimeout(2000);
  const r = await page.evaluate(() => {
    const shell = document.querySelector('.shell') as HTMLElement | null;
    const de = document.documentElement;
    return { shellZoom: shell?.style.zoom || '', overflow: de.scrollWidth - de.clientWidth };
  });
  expect(r.shellZoom, 'no inline CSS zoom on the shell in browser').toBe('');
  expect(r.overflow, 'no horizontal overflow').toBeLessThanOrEqual(2);
});

test('sidebar collapse toggle changes width (no hit-test skew)', async ({ page }) => {
  await page.goto('/#/agents');
  const sidebar = page.locator('.sidebar').first();
  await expect(sidebar).toBeVisible();
  const before = (await sidebar.boundingBox())!.width;
  await page.locator('button[aria-label="Collapse sidebar"]').first().click();
  await expect.poll(async () => (await sidebar.boundingBox())!.width).not.toBe(before);
});
