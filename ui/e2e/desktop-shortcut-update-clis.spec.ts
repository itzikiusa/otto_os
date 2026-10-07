import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ⌘U and ⌘⇧U must both fire "Update all CLIs" (keys.ts case 'u'). The shifted
// chord regressed in the exact-modifier keymap pass (3dc866e8) — users had the
// ⌘⇧U habit from before it. The spec intercepts the provider-update POST so no
// real CLI update runs; the assertion is that the chord dispatches the call
// (through the confirmation it now asks first).
//
// Desktop-browser project only (keyboard chords are a desktop concern); it
// self-skips on the mobile/tablet device projects like the other desktop specs.

let workspaceId = '';
const WORKSPACE_NAME = 'CLI shortcut workspace';

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  if (!workspaceId) {
    const { ctx, base } = await apiCtx();
    workspaceId = await seedWorkspace(ctx, base, WORKSPACE_NAME);
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
  // Boot prefetches sessions before the workspace list resolves. Only the
  // rendered current-workspace chip proves selection has reached the UI.
  await expect(page.getByTestId('agents-current-ws')).toHaveAccessibleName(
    `Current workspace: ${WORKSPACE_NAME}. Switch workspace`,
  );
});

for (const [chord, name] of [
  ['Meta+KeyU', 'Cmd+U'],
  ['Meta+Shift+KeyU', 'Cmd+Shift+U'],
] as const) {
  test(`${name} fires the update-CLIs request`, async ({ page }) => {
    await page.keyboard.press(chord);
    // Updating every agent CLI on the host asks first; nothing is sent until
    // the person confirms.
    const confirm = page.getByRole('dialog', { name: 'Update all agent CLIs?' });
    await expect(confirm).toBeVisible();
    const fired = page.waitForRequest(
      (r) => r.method() === 'POST' && new URL(r.url()).pathname === `/api/v1/workspaces/${workspaceId}/providers/update`,
      { timeout: 10_000 },
    );
    await confirm.getByRole('button', { name: 'Update', exact: true }).click();
    await fired; // resolves only if the chord dispatched updateCLIs
  });
}
