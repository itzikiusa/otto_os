import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';
import { expectDbConnected, pickDbMenuItem } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — structured query-plan panel (PlanView).
//
// Drives the REAL UI against the seeded Docker stack. The Explain toolbar button
// calls POST …/db/query-plan and renders the normalized plan as a collapsible
// op tree with red warning badges on costly access patterns:
//   • MySQL, unindexed predicate (`WHERE total_cents > 5`) → op tree + a
//     "full table scan" warning badge; Raw-JSON toggle + close both work.
//   • MySQL, primary-key lookup (`WHERE id = 1`) → op tree, NO warning badge.
//   • Redis → the Explain button is HIDDEN (capabilities.explain === false).
//
// Desktop-browser project only (the 3-pane layout). Each test self-skips when
// its container isn't reachable.
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

// Open the DB page and select a connection by name; returns once the header
// says Connected with the server version (capabilities loaded).
async function openConn(page: Page, name: string, engine: 'MySQL' | 'Redis'): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const c = page.locator('.conn-list .tr-title', { hasText: name });
  await expect(c.first()).toBeVisible({ timeout: 30_000 });
  await c.first().click();
  await expect(page.getByTestId('db-conn-state')).toBeVisible({ timeout: 20_000 });
  // The query plan's menu entry keys off capabilities.explain, so wait for
  // the capabilities before asserting it.
  await expectDbConnected(page, engine);
}

async function setEditor(page: Page, sql: string): Promise<void> {
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.press('Delete');
  await content.pressSequentially(sql, { delay: 6 });
  await page.keyboard.press('Escape'); // dismiss the autocomplete popup
  await page.waitForTimeout(200); // let the statement propagate to the store
}

// The query plan is in the workbench header's ⋯ (the Query group).
async function clickExplain(page: Page): Promise<void> {
  await pickDbMenuItem(page, /^Query plan \(EXPLAIN\)/);
  await expect(page.locator('.plan-panel')).toBeVisible({ timeout: 20_000 });
}

test.describe('DB Explorer — query-plan panel', () => {
  test('MySQL: unindexed scan → plan tree + full-scan warning; raw toggle + close', async ({ page }) => {
    test.skip(!conn.mysql, 'mysql docker not reachable');
    await openConn(page, 'e2e-mysql', 'MySQL');
    await setEditor(page, 'SELECT * FROM orders WHERE total_cents > 5');
    await clickExplain(page);

    // The plan renders as an op tree…
    expect(await page.locator('.plan-op').count()).toBeGreaterThan(0);
    // …with a red "full table scan" warning badge (orders.total_cents is unindexed).
    await expect(
      page.locator('.plan-warn', { hasText: /full table scan/i }).first(),
    ).toBeVisible({ timeout: 10_000 });

    // Raw JSON toggle swaps the tree for the engine's raw EXPLAIN JSON.
    await page.locator('.plan-btn', { hasText: 'Raw JSON' }).first().click();
    await expect(page.locator('.plan-raw')).toBeVisible();
    expect(await page.locator('.plan-op').count()).toBe(0);
    // Toggle back to the tree.
    await page.locator('.plan-btn', { hasText: 'Tree' }).first().click();
    await expect(page.locator('.plan-raw')).toBeHidden();

    // Close dismisses the panel.
    await page.locator('.plan-close').first().click();
    await expect(page.locator('.plan-panel')).toBeHidden();
  });

  test('MySQL: primary-key lookup → plan tree with NO warning badge', async ({ page }) => {
    test.skip(!conn.mysql, 'mysql docker not reachable');
    await openConn(page, 'e2e-mysql', 'MySQL');
    await setEditor(page, 'SELECT * FROM orders WHERE id = 1');
    await clickExplain(page);

    expect(await page.locator('.plan-op').count()).toBeGreaterThan(0);
    // A PK lookup is a const/index access — no full-scan (or any) warning badge.
    await expect(page.locator('.plan-warn')).toHaveCount(0);
  });

  test('Redis: Explain button is hidden (no query-plan surface)', async ({ page }) => {
    test.skip(!conn.redis, 'redis docker not reachable');
    // e2e-redis-docker is the redis connection name seeded by seedDockerConnection.
    await openConn(page, 'e2e-redis-docker', 'Redis');
    // capabilities.explain === false for Redis → the ⋯ has its Query group
    // (Run on…) but no query plan.
    const menu = page.getByRole('menu', { name: 'Context menu' });
    await expect(async () => {
      if (await menu.isVisible()) await page.keyboard.press('Escape');
      await page.getByTestId('page-header').getByRole('button', { name: 'More actions' }).first().click();
      await expect(menu.getByRole('menuitem', { name: /^Run on…/ })).toBeVisible({ timeout: 1_000 });
    }).toPass({ timeout: 20_000 });
    await expect(menu.getByRole('menuitem', { name: /^Query plan/ })).toHaveCount(0);
  });
});
