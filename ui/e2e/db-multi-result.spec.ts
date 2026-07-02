import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — multi-statement result switcher (Task 2 contract + Task 8 UI),
// against the live Docker MySQL. `SELECT …; SELECT …` returns a segmented
// switcher over the result sets; a mid-batch failure returns the completed
// results plus an errored segment (a red dot), with execution stopped there.
//
// Device-family spec (runs on the device projects; verified on iphone-portrait).
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
let connId: string | null = null;
const PHONE_MAX = 640;

test.beforeAll(async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  try {
    connId = await seedDockerConnection(ctx, base, workspaceId, 'mysql');
  } catch {
    connId = null;
  }
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

function isPhone(page: Page): boolean {
  const w = page.viewportSize()?.width ?? 0;
  return w > 0 && w <= PHONE_MAX;
}
async function ensureEditorOpen(page: Page): Promise<void> {
  if (!isPhone(page)) return;
  if (!(await page.locator('.qe-edit').isVisible().catch(() => false))) {
    await page.locator('.qe-acc-head', { hasText: 'Editor' }).click();
  }
  await expect(page.locator('.qe-edit')).toBeVisible();
}
async function ensureResultsOpen(page: Page): Promise<void> {
  if (!isPhone(page)) return;
  if (!(await page.locator('.qe-results').isVisible().catch(() => false))) {
    await page.locator('.qe-acc-head', { hasText: 'Results' }).click();
  }
  await expect(page.locator('.qe-results')).toBeVisible();
}

async function openMysql(page: Page): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const conn = page.locator('.conn-list .conn-name', { hasText: 'e2e-mysql' });
  await expect(conn.first()).toBeVisible({ timeout: 30_000 });
  await conn.first().click();
  await expect(page.locator('.main-tabs')).toBeVisible({ timeout: 20_000 });
}

async function runStatement(page: Page, sql: string): Promise<void> {
  await ensureEditorOpen(page);
  // Proven CodeMirror path (see db-sweep-mongodb's setEditorText): focus,
  // select-all, then insertText — one input event that replaces the selection,
  // so `;`-separated batches type cleanly without auto-close-bracket or
  // autocomplete interference. No separate Delete (it races the contenteditable
  // focus on WebKit and swallows the insert), and no read-back verification
  // (Playwright's text getters return empty for this CodeMirror view).
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.waitForTimeout(60);
  await page.keyboard.press('ControlOrMeta+A');
  await page.waitForTimeout(40);
  await page.keyboard.insertText(sql);
  await page.waitForTimeout(300);
  // Batches execute via "Run all" — the primary Run intentionally narrows to
  // the selection / statement under the cursor.
  await page.locator('.qe-toolbar .btn', { hasText: 'Run all' }).click();
  await expect(page.locator('.btn.small.primary', { hasText: 'Run' }).first()).toBeVisible({
    timeout: 20_000,
  });
  await ensureResultsOpen(page);
}

const segs = (page: Page) => page.locator('.rg-switch .rg-seg');

test('SELECT 1; SELECT 2 → a two-segment switcher whose value swaps', async ({ page }) => {
  test.skip(!connId, 'mysql docker not reachable');
  await openMysql(page);
  await runStatement(page, 'SELECT 111 AS a; SELECT 222 AS b');

  // Two result sets → a switcher with two segments; the first is shown.
  await expect(page.locator('.rg-switch')).toBeVisible({ timeout: 15_000 });
  await expect(segs(page)).toHaveCount(2);
  await expect(page.locator('.grid tbody')).toContainText('111');

  // Switch to Result 2 → the grid swaps to the second statement's value.
  await segs(page).nth(1).click();
  await expect(page.locator('.grid tbody')).toContainText('222');
  await expect(page.locator('.grid tbody')).not.toContainText('111');
});

test('mid-batch failure → partial results + an errored segment', async ({ page }) => {
  test.skip(!connId, 'mysql docker not reachable');
  await openMysql(page);
  // Statement 2 fails (missing table) → execution stops there: Result 1 (ok) +
  // the errored Result 2; SELECT 333 never runs.
  await runStatement(
    page,
    'SELECT 111 AS a; SELECT bad_col FROM no_such_table_xyz; SELECT 333 AS c',
  );

  await expect(page.locator('.rg-switch')).toBeVisible({ timeout: 15_000 });
  await expect(segs(page)).toHaveCount(2); // stopped at the failure — no 3rd set
  // First segment succeeded; the second carries the error badge (red dot).
  await expect(segs(page).nth(0)).not.toHaveClass(/err/);
  await expect(segs(page).nth(1)).toHaveClass(/err/);
  await expect(segs(page).nth(1).locator('.rg-seg-dot')).toBeVisible();
});
