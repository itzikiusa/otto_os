import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — file→table import dialog (Task 5.1 / Task 9 backend), against the
// live Docker stack. The dialog picks a local file on the daemon host, a target
// table, and streams batched INSERTs through the guarded write path.
//
// MySQL: CSV → an existing table → verify the row count via a follow-up query.
// MongoDB: CSV → collection via the same dialog (insertMany batches; the toolbar
//   "Import file…" shows for mongo and the tree "Import into…" covers collections).
//
// Device-family spec (verified on iphone-portrait). Fixture names are
// per-project so parallel device projects can't collide on the same table.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
const conn: Record<'mysql' | 'mongodb', string | null> = { mysql: null, mongodb: null };
let importDir = '';
let mysqlReady = false;
const PHONE_MAX = 640;
let MYSQL_TABLE = 'e2e_import_ui';
let MONGO_COLL = 'e2e_import_ui';
const MYSQL_CONTAINER = 'otto-dbv-mysql';
const MONGO_CONTAINER = 'otto-dbv-mongo';

function mysqlExec(sql: string): void {
  execFileSync('docker', ['exec', '-i', MYSQL_CONTAINER, 'mysql', '-uotto', '-pottopw', 'shopdb'], {
    input: sql,
    stdio: ['pipe', 'ignore', 'ignore'],
  });
}

function mongoExec(js: string): void {
  execFileSync(
    'docker',
    ['exec', MONGO_CONTAINER, 'mongosh', '-u', 'otto', '-p', 'ottopw',
     '--authenticationDatabase', 'admin', '--quiet', 'shopdb', '--eval', js],
    { stdio: ['ignore', 'ignore', 'ignore'] },
  );
}

test.beforeAll(async ({}, testInfo) => {
  test.setTimeout(120_000);
  // Per-project AND per-worker: fullyParallel re-runs beforeAll in each worker,
  // so a shared name would DROP/CREATE-race across workers.
  const proj = testInfo.project.name.replace(/[^a-z0-9]+/gi, '_');
  MYSQL_TABLE = `e2e_import_ui_${proj}_w${testInfo.workerIndex}`;
  MONGO_COLL = `e2e_import_ui_${proj}_w${testInfo.workerIndex}`;
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

  // A CSV fixture on the daemon host (same machine as the test).
  importDir = mkdtempSync(join(tmpdir(), 'otto-e2e-dbimport-'));
  writeFileSync(join(importDir, 'people.csv'), 'id,name\n1,Ada\n2,Grace\n3,Linus\n');

  // A fresh, empty MySQL target table for the import to fill.
  if (conn.mysql) {
    try {
      mysqlExec(
        `DROP TABLE IF EXISTS ${MYSQL_TABLE}; CREATE TABLE ${MYSQL_TABLE} (id INT, name VARCHAR(50));`,
      );
      mysqlReady = true;
    } catch {
      mysqlReady = false;
    }
  }
  // A clean Mongo target collection (drops leftovers from prior runs).
  if (conn.mongodb) {
    try {
      mongoExec(`db.${MONGO_COLL}.drop()`);
    } catch {
      /* best-effort — the count assertion below is what really guards this */
    }
  }
});

test.afterAll(async () => {
  if (mysqlReady) {
    try {
      mysqlExec(`DROP TABLE IF EXISTS ${MYSQL_TABLE};`);
    } catch {
      /* best-effort */
    }
  }
  if (conn.mongodb) {
    try {
      mongoExec(`db.${MONGO_COLL}.drop()`);
    } catch {
      /* best-effort */
    }
  }
  try {
    rmSync(importDir, { recursive: true, force: true });
  } catch {
    /* best-effort */
  }
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

async function openConn(page: Page, name: string): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const c = page.locator('.conn-list .conn-name', { hasText: name });
  await expect(c.first()).toBeVisible({ timeout: 30_000 });
  await c.first().click();
  await expect(page.locator('.main-tabs')).toBeVisible({ timeout: 20_000 });
}

async function runStatement(page: Page, sql: string): Promise<void> {
  await ensureEditorOpen(page);
  // Proven CodeMirror path (see db-sweep-mongodb's setEditorText): focus,
  // select-all, then insertText — replaces the selection in one input event.
  // No separate Delete (races the contenteditable focus on WebKit and swallows
  // the insert); no read-back verification (text getters return empty for this
  // CodeMirror view). The settle lets the value reach the store before Run.
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

test('MySQL: import a CSV into a table via the dialog', async ({ page }) => {
  test.skip(!conn.mysql || !mysqlReady, 'mysql connection / fixture unavailable');
  await openConn(page, 'e2e-mysql');
  // Run any query so the grid toolbar (with "Import file…") is present.
  await runStatement(page, 'SELECT * FROM customers ORDER BY id');

  await page.locator('.grid-toolbar .tb-btn', { hasText: 'Import file' }).click();
  await expect(page.locator('.imp-form')).toBeVisible({ timeout: 10_000 });
  await page.locator('input[placeholder="~/Downloads/data.csv"]').fill(join(importDir, 'people.csv'));
  await page.locator('input[placeholder="target_table"]').fill(MYSQL_TABLE);
  // Format defaults to CSV. Submit.
  await page.getByRole('button', { name: 'Import', exact: true }).click();

  // Streamed {done} line → success toast with the row/batch summary.
  await expect(page.locator('.toast.success', { hasText: 'Imported' })).toBeVisible({
    timeout: 30_000,
  });

  // Verify the rows actually landed.
  await runStatement(page, `SELECT COUNT(*) AS c FROM ${MYSQL_TABLE}`);
  await expect(page.locator('.grid tbody')).toContainText('3');
});

test('MongoDB: import a CSV into a collection via the dialog', async ({ page }) => {
  test.skip(!conn.mongodb, 'mongodb docker not reachable');
  await openConn(page, 'e2e-mongodb');
  // Mongo find to surface the results toolbar (if an import entry existed).
  await runStatement(page, 'db.orders.find({})');
  const importBtn = page.locator('.grid-toolbar .tb-btn', { hasText: 'Import file' });
  await expect(importBtn).toBeVisible({ timeout: 10_000 });
  await importBtn.click();
  await expect(page.locator('.imp-form')).toBeVisible({ timeout: 10_000 });
  await page.locator('input[placeholder="~/Downloads/data.csv"]').fill(join(importDir, 'people.csv'));
  await page.locator('input[placeholder="target_table"]').fill(MONGO_COLL);
  await page.getByRole('button', { name: 'Import', exact: true }).click();
  await expect(page.locator('.toast.success', { hasText: 'Imported' })).toBeVisible({
    timeout: 30_000,
  });
  await runStatement(page, `db.${MONGO_COLL}.find({})`);
  await expect(page.locator('.grid tbody tr:not(.spacer)')).toHaveCount(3, { timeout: 10_000 });
});
