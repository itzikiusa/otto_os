import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — workbench restore on reload (Task 7). Open TWO connections
// (MySQL + Redis, live Docker), switch to the second, pick a per-connection main
// view, then reload the page and assert the workbench comes back:
//   • both connection tabs restored,
//   • the previously-focused connection is still selected,
//   • its per-connection main tab (Structure) is restored,
//   • no uncaught page errors during the reload/restore.
//
// Desktop-browser project only; skips when either engine is down.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
const conn: Record<'mysql' | 'redis', string | null> = { mysql: null, redis: null };

test.beforeAll(async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  for (const k of ['mysql', 'redis'] as const) {
    try {
      conn[k] = await seedDockerConnection(ctx, base, workspaceId, k);
    } catch {
      conn[k] = null;
    }
  }
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

/** Open a connection from the sidebar picker. Opening the first connection flips
 *  the sidebar to the schema view, so re-show the "Connections" tab first. */
async function openConnFromList(page: Page, name: string): Promise<void> {
  const connTab = page.locator('.side-switch .ss', { hasText: 'Connections' });
  if (await connTab.first().isVisible().catch(() => false)) await connTab.first().click();
  const c = page.locator('.conn-list .conn-name', { hasText: name });
  await expect(c.first()).toBeVisible({ timeout: 30_000 });
  await c.first().click();
  await expect(page.locator('.main-tabs')).toBeVisible({ timeout: 20_000 });
  await expect(
    page.locator('.conn-tab.active .conn-tab-name', { hasText: name }),
  ).toBeVisible({ timeout: 20_000 });
}

test('workbench restores open tabs + selection + per-connection view on reload', async ({
  page,
}) => {
  test.skip(!conn.mysql || !conn.redis, 'mysql + redis docker not both reachable');

  const pageErrors: string[] = [];
  page.on('pageerror', (e) => pageErrors.push(String(e)));

  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });

  // Open MySQL, then Redis (Redis becomes the focused tab).
  await openConnFromList(page, 'e2e-mysql');
  await openConnFromList(page, 'e2e-redis-docker');
  await expect(page.locator('.conn-tab')).toHaveCount(2);
  await expect(page.locator('.conn-tab.active .conn-tab-name')).toHaveText('e2e-redis-docker');

  // Choose a per-connection main view on the focused (Redis) connection.
  await page.locator('.main-tabs .mt', { hasText: 'Structure' }).first().click();
  await expect(page.locator('.main-tabs .mt.active')).toHaveText('Structure');
  // Let the localStorage writes (open set + per-conn view) settle before reload.
  await page.waitForTimeout(400);

  // Reload — restoreWorkbench must re-open both tabs, keep the selection, and
  // restore the focused connection's main view.
  await page.reload();
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });

  await expect(page.locator('.conn-tab')).toHaveCount(2, { timeout: 25_000 });
  await expect(page.locator('.conn-tab.active .conn-tab-name')).toHaveText('e2e-redis-docker', {
    timeout: 25_000,
  });
  await expect(page.locator('.main-tabs .mt.active')).toHaveText('Structure', { timeout: 25_000 });

  // Both connection names present (order preserved: mysql first, redis second).
  await expect(page.locator('.conn-tab .conn-tab-name').nth(0)).toHaveText('e2e-mysql');
  await expect(page.locator('.conn-tab .conn-tab-name').nth(1)).toHaveText('e2e-redis-docker');

  expect(pageErrors, `uncaught page errors during restore: ${pageErrors.join(' | ')}`).toEqual([]);
});
