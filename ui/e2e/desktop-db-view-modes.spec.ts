import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — result view modes (A1 / A1b) + pinned query tabs (A4).
//
// The view a result renders in is layered (`effectiveViewMode`): an explicit
// pick on the tab → the auto-Vertical column threshold → the view remembered
// for the connection → the engine default (Mongo = Vertical, SQL = Grid).
// Against the live Docker stack (MySQL + MongoDB) this proves:
//   • a Mongo `find` opens in Vertical with nothing picked;
//   • ⇧⌘V cycles Grid → Vertical → JSON → Grid, the pick is stored on the tab
//     and survives a reload (with the "Auto" chip offering the way back);
//   • a result wider than the threshold auto-verticals; picking Grid overrides
//     it for THAT tab only — a new tab is automatic again;
//   • a pinned tab survives "Close all" and a reload.
//
// Desktop-browser project only; each test skips when its engine is down.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
const conn: Record<'mysql' | 'mongodb', string | null> = { mysql: null, mongodb: null };

test.beforeAll(async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  for (const k of ['mysql', 'mongodb'] as const) {
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

async function openConn(page: Page, name: string): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const c = page.locator('.conn-list .conn-name', { hasText: name });
  await expect(c.first()).toBeVisible({ timeout: 30_000 });
  await c.first().click();
  await expect(page.locator('.main-tabs')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.query-editor')).toBeVisible({ timeout: 15_000 });
}

/** Put a statement into the editor and run it. `insertText` is one input event
 *  (the server-backed autocomplete can't corrupt a dotted Mongo command); the
 *  settle lets the value reach the store before Run reads it. Resolves once the
 *  results toolbar (with the view switch) is on screen. */
async function runStatement(page: Page, stmt: string): Promise<void> {
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText(stmt);
  await page.waitForTimeout(300);
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
  await expect(page.locator('.view-seg')).toBeVisible({ timeout: 30_000 });
}

const activeView = (page: Page) => page.locator('.view-seg .vs.on');
const autoChip = (page: Page) => page.locator('.view-seg .vs.auto');

test('mongo find defaults to Vertical', async ({ page }) => {
  test.skip(!conn.mongodb, 'mongodb docker not reachable');
  await openConn(page, 'e2e-mongodb');
  await runStatement(page, 'db.customers.find({})');
  await expect(activeView(page)).toHaveText('Vertical');
  await expect(page.locator('.vrec').first()).toBeVisible({ timeout: 10_000 });
  // Nothing was picked, so there is nothing to clear.
  await expect(autoChip(page)).toHaveCount(0);
});

test('⌘⇧V cycles and the pick survives reload', async ({ page }) => {
  test.skip(!conn.mongodb, 'mongodb docker not reachable');
  await openConn(page, 'e2e-mongodb');
  await runStatement(page, 'db.customers.find({})');
  await expect(activeView(page)).toHaveText('Vertical');

  // Focus is on the Run button (inside .query-editor) — the chord is DB-scoped.
  await page.keyboard.press('Meta+Shift+V');
  await expect(activeView(page)).toHaveText('JSON', { timeout: 5_000 });
  await expect(page.locator('.jrec').first()).toBeVisible();
  await page.keyboard.press('Meta+Shift+V');
  await expect(activeView(page)).toHaveText('Grid', { timeout: 5_000 });
  await expect(page.locator('.grid tbody tr:not(.spacer)').first()).toBeVisible();
  // An explicit pick exposes the way back to automatic.
  await expect(autoChip(page)).toBeVisible();

  // Let the debounced tab persist settle, then reload: the statement and the
  // pick come back (results never persist, so re-run to see the grid).
  await page.waitForTimeout(500);
  await page.reload();
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.query-editor')).toBeVisible({ timeout: 20_000 });
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
  await expect(page.locator('.view-seg')).toBeVisible({ timeout: 30_000 });
  await expect(activeView(page)).toHaveText('Grid');
  await expect(autoChip(page)).toBeVisible();

  // "Auto" clears the tab's pick → back to the engine default (Vertical).
  await autoChip(page).click();
  await expect(activeView(page)).toHaveText('Vertical');
  await expect(autoChip(page)).toHaveCount(0);
});

test('threshold: wide result auto-verticals, explicit pick wins for that tab only', async ({
  page,
}) => {
  test.skip(!conn.mysql, 'mysql docker not reachable');
  // The user-side setting: more than 2 columns → Vertical.
  await page.addInitScript(() => localStorage.setItem('otto_db_auto_vertical_cols', '2'));
  await openConn(page, 'e2e-mysql');
  await runStatement(page, 'SELECT 1 AS a, 2 AS b, 3 AS c');
  await expect(activeView(page)).toHaveText('Vertical');
  await expect(page.locator('.view-seg')).toHaveAttribute('title', /auto: 3 columns > 2/);

  // Picking Grid overrides the threshold for THIS tab (and offers Auto).
  await page.locator('.view-seg .vs', { hasText: 'Grid' }).click();
  await expect(activeView(page)).toHaveText('Grid');
  await expect(page.locator('.grid tbody tr:not(.spacer)').first()).toBeVisible();
  await expect(autoChip(page)).toBeVisible();

  // A new tab has no pick: the threshold applies again — even though Grid is
  // now the connection's remembered view (the threshold outranks it).
  await page.locator('.qe-tab-new').click();
  await expect(page.locator('.qe-tabs .qe-tab')).toHaveCount(2);
  await runStatement(page, 'SELECT 1 AS a, 2 AS b, 3 AS c');
  await expect(activeView(page)).toHaveText('Vertical');
  await expect(autoChip(page)).toHaveCount(0);

  // A narrow result on that same tab falls through to the connection memory.
  await runStatement(page, 'SELECT 1 AS a');
  await expect(activeView(page)).toHaveText('Grid');
  await expect(page.locator('.view-seg')).toHaveAttribute('title', 'remembered for this connection');
});

test('pinned tab survives close-all', async ({ page }) => {
  test.skip(!conn.mysql, 'mysql docker not reachable');
  await openConn(page, 'e2e-mysql');
  const tabs = page.locator('.qe-tabs .qe-tab');
  await expect(tabs).toHaveCount(1);

  // Give the tab a buffer so it is recognisable after the reload, then pin it.
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.insertText('SELECT 42 AS pinned');
  await page.waitForTimeout(300);
  await tabs.first().click({ button: 'right' });
  await expect(page.locator('.ctx-menu')).toBeVisible();
  await page.locator('.ctx-item', { hasText: /^Pin tab$/ }).click();
  await expect(page.locator('.ctx-menu')).toBeHidden();
  await expect(tabs.first().locator('.qe-tab-pin')).toBeVisible();

  // A second (unpinned) tab. The pinned tab has no × even now that there are
  // two tabs, and ⌥⌘W on it refuses with a toast (the tab count doesn't change).
  await page.locator('.qe-tab-new').click();
  await expect(tabs).toHaveCount(2);
  await expect(tabs.first().locator('.qe-tab-close')).toHaveCount(0);
  await expect(tabs.nth(1).locator('.qe-tab-close')).toHaveCount(1);
  await tabs.first().click();
  await expect(tabs.first()).toHaveClass(/active/);
  await content.click();
  await page.keyboard.press('Meta+Alt+KeyW');
  await expect(page.locator('.toast.info', { hasText: 'Unpin to close' })).toBeVisible({
    timeout: 5_000,
  });
  await expect(tabs).toHaveCount(2);

  // "Close all" from the unpinned tab's menu keeps the pinned one.
  await tabs.nth(1).click({ button: 'right' });
  await expect(page.locator('.ctx-menu')).toBeVisible();
  await expect(page.locator('.ctx-item', { hasText: /^Close all/ })).toHaveText(
    /keeps pinned tabs/,
  );
  await page.locator('.ctx-item', { hasText: /^Close all/ }).click();
  await expect(page.locator('.ctx-menu')).toBeHidden();

  // Only the pinned tab is left, still pinned, still not closable.
  await expect(tabs).toHaveCount(1);
  await expect(tabs.first().locator('.qe-tab-pin')).toBeVisible();
  await expect(tabs.first().locator('.qe-tab-close')).toHaveCount(0);
  await expect(page.locator('.qe-tab-label')).toHaveText(/pinned|SELECT/i);

  // The pin is persisted with the tab.
  await page.waitForTimeout(500);
  await page.reload();
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.query-editor')).toBeVisible({ timeout: 20_000 });
  await expect(tabs).toHaveCount(1);
  await expect(tabs.first().locator('.qe-tab-pin')).toBeVisible();
  await expect(page.locator('.qe-tab-label')).toHaveText(/pinned|SELECT/i);
});
