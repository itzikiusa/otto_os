import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

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

// Open the DB page and select a connection by name; leaves the engine chip
// (capabilities loaded) visible so downstream assertions are stable.
async function openConn(page: Page, name: string, engine: string): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const c = page.locator('.conn-list .conn-name', { hasText: name });
  await expect(c.first()).toBeVisible({ timeout: 30_000 });
  await c.first().click();
  await expect(page.locator('.main-tabs')).toBeVisible({ timeout: 20_000 });
  // The cap-chip confirms capabilities loaded — the Explain button's visibility
  // keys off capabilities.explain, so we must wait for that before asserting it.
  await expect(page.locator('.cap-chip', { hasText: engine })).toBeVisible({ timeout: 20_000 });
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

const explainBtn = (page: Page) => page.locator('.btn.small.ghost', { hasText: 'Explain' });

async function clickExplain(page: Page): Promise<void> {
  await explainBtn(page).first().click();
  await expect(page.locator('.plan-panel')).toBeVisible({ timeout: 20_000 });
}

test.describe('DB Explorer — query-plan panel', () => {
  test('MySQL: unindexed scan → plan tree + full-scan warning; raw toggle + close', async ({ page }) => {
    test.skip(!conn.mysql, 'mysql docker not reachable');
    await openConn(page, 'e2e-mysql', 'mysql');
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
    await openConn(page, 'e2e-mysql', 'mysql');
    await setEditor(page, 'SELECT * FROM orders WHERE id = 1');
    await clickExplain(page);

    expect(await page.locator('.plan-op').count()).toBeGreaterThan(0);
    // A PK lookup is a const/index access — no full-scan (or any) warning badge.
    await expect(page.locator('.plan-warn')).toHaveCount(0);
  });

  test('Redis: Explain button is hidden (no query-plan surface)', async ({ page }) => {
    test.skip(!conn.redis, 'redis docker not reachable');
    // e2e-redis-docker is the redis connection name seeded by seedDockerConnection.
    await openConn(page, 'e2e-redis-docker', 'redis');
    // capabilities.explain === false for Redis → the Explain button is not rendered.
    await expect(explainBtn(page)).toHaveCount(0);
  });
});
