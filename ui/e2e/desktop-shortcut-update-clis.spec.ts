import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ⌘U and ⌘⇧U must both fire "Update all CLIs" (keys.ts case 'u'). The shifted
// chord regressed in the exact-modifier keymap pass (3dc866e8) — users had the
// ⌘⇧U habit from before it. The spec intercepts the provider-update POST so no
// real CLI update runs; the assertion is that the chord dispatches the call.
//
// Desktop-browser project only (keyboard chords are a desktop concern); it
// self-skips on the mobile/tablet device projects like the other desktop specs.

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
  await page.route('**/providers/update', (route) =>
    route.fulfill({ status: 503, contentType: 'text/plain', body: 'e2e-intercepted' }),
  );
  await page.goto('/#/agents');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  // updateAllCLIs bails ("No workspace selected") until the workspace store
  // resolves ws.currentId — its persisted localStorage write marks that point.
  await expect
    .poll(async () => page.evaluate(() => localStorage.getItem('otto_workspace')))
    .toBeTruthy();
});

for (const [chord, name] of [
  ['Meta+KeyU', 'Cmd+U'],
  ['Meta+Shift+KeyU', 'Cmd+Shift+U'],
] as const) {
  test(`${name} fires the update-CLIs request`, async ({ page }) => {
    const fired = page.waitForRequest(
      (r) => r.method() === 'POST' && r.url().includes('/providers/update'),
      { timeout: 10_000 },
    );
    await page.keyboard.press(chord);
    await fired; // resolves only if the chord dispatched updateCLIs
  });
}
