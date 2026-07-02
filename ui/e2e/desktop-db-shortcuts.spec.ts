import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — keyboard shortcuts + running overlay (Task 8 / keys.ts fix).
// Desktop-browser only. Verifies the DB-scoped chords fire WITHOUT leaking to the
// shell (no session modal / palette): ⌘S opens the save bar, ⌥⌘T new query tab,
// ⌥⌘W close query tab, ⌥⌘→/← switch tabs, and Esc cancels a running query while
// the running overlay is visible.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
let mysqlConn: string | null = null;

test.beforeAll(async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  try {
    mysqlConn = await seedDockerConnection(ctx, base, workspaceId, 'mysql');
  } catch {
    mysqlConn = null;
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

async function openMysql(page: Page): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const conn = page.locator('.conn-list .conn-name', { hasText: 'e2e-mysql' });
  await expect(conn.first()).toBeVisible({ timeout: 30_000 });
  await conn.first().click();
  await expect(page.locator('.main-tabs')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.query-editor')).toBeVisible({ timeout: 15_000 });
}

/** Put a statement into the editor (focus lands inside `.query-editor`, which is
 *  what the shortcut handler requires). */
async function typeStatement(page: Page, sql: string): Promise<void> {
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.press('Delete');
  await page.keyboard.insertText(sql);
}

test('⌘S opens the save bar — no shell side-effects', async ({ page }) => {
  test.skip(!mysqlConn, 'mysql docker not reachable');
  await openMysql(page);
  await typeStatement(page, 'SELECT 1');
  await page.keyboard.press('Meta+KeyS');
  await expect(page.locator('.save-bar')).toBeVisible({ timeout: 5_000 });
  // The save bar is the DB save UI, not a browser "save page" or a shell modal.
  await expect(page).toHaveURL(/#\/database/);
});

test('⌥⌘T adds a query tab, ⌥⌘W closes it — not shell sessions', async ({ page }) => {
  test.skip(!mysqlConn, 'mysql docker not reachable');
  await openMysql(page);
  const tabs = page.locator('.qe-tabs .qe-tab');
  await expect(tabs).toHaveCount(1);

  await page.locator('.qe-edit .cm-content').click();
  await page.keyboard.press('Meta+Alt+KeyT');
  await expect(tabs).toHaveCount(2, { timeout: 5_000 });
  // A DB QUERY tab was added — the shell's ⌘T (new session) did NOT fire: no
  // session modal / command palette opened, and we're still on the DB view.
  await expect(page.locator('.palette, .session-create, .cmd-palette')).toHaveCount(0);
  await expect(page).toHaveURL(/#\/database/);

  await page.locator('.qe-edit .cm-content').click();
  await page.keyboard.press('Meta+Alt+KeyW');
  await expect(tabs).toHaveCount(1, { timeout: 5_000 });
});

test('⌥⌘→ / ⌥⌘← switch query tabs', async ({ page }) => {
  test.skip(!mysqlConn, 'mysql docker not reachable');
  await openMysql(page);
  // Open a second tab (now active = index 1).
  await page.locator('.qe-tab-new').click();
  const tabs = page.locator('.qe-tabs .qe-tab');
  await expect(tabs).toHaveCount(2);
  await expect(tabs.nth(1)).toHaveClass(/active/);

  await page.locator('.qe-edit .cm-content').click();
  await page.keyboard.press('Meta+Alt+ArrowLeft');
  await expect(tabs.nth(0)).toHaveClass(/active/, { timeout: 5_000 });

  await page.keyboard.press('Meta+Alt+ArrowRight');
  await expect(tabs.nth(1)).toHaveClass(/active/, { timeout: 5_000 });
});

test('running overlay shows during a slow query; Esc cancels it', async ({ page }) => {
  test.skip(!mysqlConn, 'mysql docker not reachable');
  await openMysql(page);
  await typeStatement(page, 'SELECT SLEEP(5)');
  // Kick the run — the Run button flips to Stop while in flight.
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();

  // The running overlay (dimmed grid + elapsed counter + Cancel) is visible.
  await expect(page.locator('.rg-overlay')).toBeVisible({ timeout: 10_000 });
  await expect(page.locator('.rg-overlay-text')).toContainText(/Running…/);

  // Esc cancels the in-flight query (engine KILL + client abort) well before the
  // 5s SLEEP would finish — the overlay clears and the Run button returns.
  await page.locator('.qe-edit .cm-content').click();
  await page.keyboard.press('Escape');
  await expect(page.locator('.rg-overlay')).toHaveCount(0, { timeout: 8_000 });
  await expect(page.locator('.btn.small.primary', { hasText: 'Run' }).first()).toBeVisible({
    timeout: 8_000,
  });
});
