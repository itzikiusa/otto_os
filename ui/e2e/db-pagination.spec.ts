import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — footer pager (Task 4.7 / Task 8 UI) against the live Docker
// MySQL. An auto-limited bare SELECT shows a pager (‹ Prev · rows a–b · Next ›)
// that re-runs with server OFFSET; an explicit user LIMIT disables it.
//
// The default row cap is set to 25 via localStorage (the UI select's smallest
// option is 100, but the store reads `otto_db_row_limit`), so a 100-row fixture
// paginates in 25s. Device-family spec (verified on iphone-portrait).
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
let connId: string | null = null;
let ready = false;
const PHONE_MAX = 640;
// fullyParallel puts same-file tests in DIFFERENT workers, each re-running
// beforeAll — the fixture name must be per-project AND per-worker or the
// concurrent DROP/CREATE races leave one worker with a missing table.
let TABLE = 'e2e_page';
const MYSQL_CONTAINER = 'otto-dbv-mysql';

function mysqlExec(sql: string): void {
  execFileSync('docker', ['exec', '-i', MYSQL_CONTAINER, 'mysql', '-uotto', '-pottopw', 'shopdb'], {
    input: sql,
    stdio: ['pipe', 'ignore', 'ignore'],
  });
}

test.beforeAll(async ({}, testInfo) => {
  test.setTimeout(120_000);
  const proj = testInfo.project.name.replace(/[^a-z0-9]+/gi, '_');
  TABLE = `e2e_page_${proj}_w${testInfo.workerIndex}`;
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  try {
    connId = await seedDockerConnection(ctx, base, workspaceId, 'mysql');
  } catch {
    connId = null;
  }
  await ctx.dispose().catch(() => {});

  if (connId) {
    // 100 rows (ids 0..99) via a 10×10 cross join — enough for several 25-row pages.
    const d10 =
      '(SELECT 0 n UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL ' +
      'SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL ' +
      'SELECT 8 UNION ALL SELECT 9)';
    try {
      mysqlExec(
        `DROP TABLE IF EXISTS ${TABLE};` +
          `CREATE TABLE ${TABLE} (id INT PRIMARY KEY, v INT);` +
          `INSERT INTO ${TABLE} (id, v) SELECT a.n + b.n*10, a.n FROM ${d10} a CROSS JOIN ${d10} b;`,
      );
      ready = true;
    } catch {
      ready = false;
    }
  }
});

test.afterAll(async () => {
  if (ready) {
    try {
      mysqlExec(`DROP TABLE IF EXISTS ${TABLE};`);
    } catch {
      /* best-effort */
    }
  }
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
    // Page size 25 (below the UI select's 100 minimum, but the store honours it).
    localStorage.setItem('otto_db_row_limit', '25');
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
  // select-all, then insertText — which REPLACES the selection in one input
  // event. No separate Delete (it races the contenteditable focus on WebKit
  // and swallows the following insert), and no read-back verification
  // (Playwright's text getters return empty for this CodeMirror view even
  // when it holds text). The settle lets the value reach the store before Run.
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.waitForTimeout(60);
  await page.keyboard.press('ControlOrMeta+A');
  await page.waitForTimeout(40);
  await page.keyboard.insertText(sql);
  await page.waitForTimeout(300);
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
  await expect(page.locator('.btn.small.primary', { hasText: 'Run' }).first()).toBeVisible({
    timeout: 20_000,
  });
  await ensureResultsOpen(page);
  await expect(page.locator('.grid tbody tr:not(.spacer)').first()).toBeVisible({ timeout: 20_000 });
}

// The pager range text uses an en-dash; accept either dash.
const P1 = /rows\s*1[–-]25/;
const P2 = /rows\s*26[–-]50/;

test('auto-limited SELECT shows a pager; Next/Prev page the result', async ({ page }) => {
  test.skip(!connId || !ready, 'mysql connection / fixture unavailable');
  await openMysql(page);
  await runStatement(page, `SELECT * FROM ${TABLE} ORDER BY id`);

  // Page 1: pager present (result was auto-limited to 25), rows 1–25.
  const pager = page.locator('.grid-foot .pager');
  await expect(pager).toBeVisible({ timeout: 10_000 });
  await expect(page.locator('.pg-range')).toContainText(P1);

  // Next → rows 26–50 (server re-ran with OFFSET 25).
  await page.locator('.pg-btn', { hasText: 'Next' }).click();
  await expect(page.locator('.pg-range')).toContainText(P2, { timeout: 10_000 });

  // Prev → back to rows 1–25.
  await page.locator('.pg-btn', { hasText: 'Prev' }).click();
  await expect(page.locator('.pg-range')).toContainText(P1, { timeout: 10_000 });
});

test('explicit user LIMIT disables the pager', async ({ page }) => {
  test.skip(!connId || !ready, 'mysql connection / fixture unavailable');
  await openMysql(page);
  // An explicit LIMIT makes the auto-limiter bail → no auto_limited → no pager.
  await runStatement(page, `SELECT * FROM ${TABLE} ORDER BY id LIMIT 10`);
  await expect(page.locator('.grid-foot')).toBeVisible({ timeout: 10_000 });
  await expect(page.locator('.grid-foot .pager')).toHaveCount(0);
});
