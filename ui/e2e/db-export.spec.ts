import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — full export + streamed progress (against the live Docker MySQL
// stack 127.0.0.1:13306 · otto/ottopw · shopdb). Skips cleanly when the dev DB
// stack isn't up (like the db-sweep-* specs).
//
// Verifies the two fixes:
//   1. The confusing capped "Full Export" button is gone; one clear
//      "Export all rows…" opens the dialog and exports the FULL result to a file.
//   2. The export streams NDJSON progress: a large export shows a live progress
//      bar with a growing byte counter, never idling out, and finishes with a
//      success toast carrying the streamed row/byte summary.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
let connId: string | null = null;
let exportDir = '';
let bigTableReady = false;
const PHONE_MAX = 640;
const BIG_TABLE = 'e2e_export_big';
const MYSQL_CONTAINER = 'otto-dbv-mysql';

/** Run SQL straight in the MySQL container (bypassing the daemon). */
function mysqlExec(sql: string): void {
  execFileSync('docker', ['exec', '-i', MYSQL_CONTAINER, 'mysql', '-uotto', '-pottopw', 'shopdb'], {
    input: sql,
    stdio: ['pipe', 'ignore', 'ignore'],
  });
}

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
  exportDir = mkdtempSync(join(tmpdir(), 'otto-e2e-dbexport-'));

  // Build a large fixture table so the streaming export spans multiple progress
  // ticks (≥300ms) and the bar/byte-counter is genuinely observable. A 5-way
  // cross join of a 10-row inline base → exactly 100,000 rows (~19MB export).
  // Built DIRECTLY in the container — fast + reliable; the daemon path we test is
  // the export READ, not fixture writes.
  if (connId) {
    const d10 =
      '(SELECT 0 n UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL ' +
      'SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL ' +
      'SELECT 8 UNION ALL SELECT 9)';
    try {
      mysqlExec(
        `DROP TABLE IF EXISTS ${BIG_TABLE};` +
          `CREATE TABLE ${BIG_TABLE} (id INT, payload VARCHAR(200));` +
          `INSERT INTO ${BIG_TABLE} (id, payload) ` +
          `SELECT a.n + b.n*10 + c.n*100 + d.n*1000 + e.n*10000, REPEAT('x', 180) ` +
          `FROM ${d10} a CROSS JOIN ${d10} b CROSS JOIN ${d10} c ` +
          `CROSS JOIN ${d10} d CROSS JOIN ${d10} e;`,
      );
      bigTableReady = true;
    } catch {
      bigTableReady = false; // docker unavailable → the large-export test skips
    }
  }
});

test.afterAll(async () => {
  if (bigTableReady) {
    try {
      mysqlExec(`DROP TABLE IF EXISTS ${BIG_TABLE};`);
    } catch {
      /* best-effort cleanup */
    }
  }
  try {
    rmSync(exportDir, { recursive: true, force: true });
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

async function openMysql(page: Page): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const conn = page.locator('.conn-list .conn-name', { hasText: 'e2e-mysql' });
  await expect(conn.first()).toBeVisible({ timeout: 30_000 });
  await conn.first().click();
  await expect(page.locator('.main-tabs')).toBeVisible({ timeout: 20_000 });
}

async function ensureEditorOpen(page: Page): Promise<void> {
  if (!isPhone(page)) return;
  const editor = page.locator('.qe-edit');
  if (!(await editor.isVisible().catch(() => false))) {
    await page.locator('.qe-acc-head', { hasText: 'Editor' }).click();
  }
  await expect(editor).toBeVisible();
}

async function ensureResultsOpen(page: Page): Promise<void> {
  if (!isPhone(page)) return;
  const results = page.locator('.qe-results');
  if (!(await results.isVisible().catch(() => false))) {
    await page.locator('.qe-acc-head', { hasText: 'Results' }).click();
  }
  await expect(results).toBeVisible();
}

async function runRead(page: Page, sql: string): Promise<void> {
  await ensureEditorOpen(page);
  const content = page.locator('.qe-edit .cm-content');
  const mod = process.platform === 'darwin' ? 'Meta' : 'Control';
  const want = sql.replace(/\s+/g, ' ').trim();
  for (let attempt = 0; attempt < 3; attempt++) {
    await content.click();
    await expect(content).toBeFocused({ timeout: 5_000 });
    await page.keyboard.press(`${mod}+A`);
    await page.keyboard.press('Delete');
    await content.pressSequentially(sql, { delay: 8 });
    await page.keyboard.press('Escape');
    const got = ((await content.textContent()) ?? '').replace(/\s+/g, ' ').trim();
    if (got.startsWith(want)) break;
  }
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
  await expect(page.locator('.btn.small.primary', { hasText: 'Run' }).first()).toBeVisible({ timeout: 20_000 });
  await ensureResultsOpen(page);
  await expect(page.locator('.grid tbody tr:not(.spacer)').first()).toBeVisible({ timeout: 20_000 });
}

/** Open the export dialog and point it at our isolated temp dir + a unique name. */
async function openExportDialog(page: Page, fileName: string): Promise<void> {
  await page.locator('.tb-btn', { hasText: 'Export all rows' }).first().click();
  await expect(page.locator('.exp-form')).toBeVisible({ timeout: 10_000 });
  await page.locator('.exp-input[placeholder="~/Downloads"]').fill(exportDir);
  await page.locator('.exp-input[placeholder="result.csv"]').fill(fileName);
}

test.describe('DB Explorer · export', () => {
  // Serial: keep all tests in ONE worker so the file-level beforeAll (which does
  // shared-DB DDL to build the fixture table) runs once, with no cross-worker
  // race on CREATE/DROP/INSERT of the same table.
  test.describe.configure({ mode: 'serial' });

  test('connection seeds & is reachable', () => {
    expect(
      connId,
      'seedDockerConnection(mysql) returned null — MySQL not reachable at 127.0.0.1:13306',
    ).not.toBeNull();
  });

  test('"Export all rows…" replaces the capped Full Export and writes the full file', async ({
    page,
  }, info) => {
    test.skip(connId == null, 'mysql connection unavailable');
    await openMysql(page);
    await runRead(page, 'SELECT * FROM customers ORDER BY id');

    // Fix #1: one clear full-export control; the misleading "Full Export" is gone.
    await expect(page.locator('.grid-toolbar')).toContainText('Export all rows');
    await expect(page.locator('.grid-toolbar .tb-btn', { hasText: 'Full Export' })).toHaveCount(0);

    const fileName = `customers-${info.project.name}.csv`;
    await openExportDialog(page, fileName);
    await page.getByRole('button', { name: /^Export all$/ }).click();

    // The streamed final line drives a success toast with the row/byte summary.
    await expect(page.locator('.toast.success .toast-title', { hasText: 'Exported' })).toBeVisible({
      timeout: 30_000,
    });

    // The full result was actually written to the chosen file on the daemon host.
    const dest = join(exportDir, fileName);
    expect(existsSync(dest), `export file ${dest} should exist`).toBe(true);
    const body = readFileSync(dest, 'utf8');
    expect(body.length, 'export file should not be empty').toBeGreaterThan(0);
    expect(body, 'export should contain the seeded customer').toContain('ada@example.com');
  });

  test('large export shows a live progress bar then finishes', async ({ page }, info) => {
    test.setTimeout(90_000);
    test.skip(connId == null || !bigTableReady, 'mysql connection / fixture unavailable');
    await openMysql(page);
    // The display SELECT is row-capped, but the export re-runs it UNCAPPED, so it
    // streams the whole 150k-row table — long enough to span progress ticks.
    await runRead(page, `SELECT * FROM ${BIG_TABLE}`);

    const fileName = `big-${info.project.name}.csv`;
    await openExportDialog(page, fileName);
    await page.getByRole('button', { name: /^Export all$/ }).click();

    // Fix #2: the progress region appears with a live "bytes written" readout
    // while the export streams (no frozen spinner / idle timeout).
    await expect(page.locator('.exp-progress .exp-bar')).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('.exp-prog-text')).toContainText(/written…/, { timeout: 15_000 });

    // …and it completes with the streamed summary.
    await expect(page.locator('.toast.success .toast-title', { hasText: 'Exported' })).toBeVisible({
      timeout: 60_000,
    });

    const dest = join(exportDir, fileName);
    expect(existsSync(dest)).toBe(true);
    // A genuinely full export of 150k×~190-byte rows is multiple MB — proof it
    // wasn't silently capped at the old 1000-row default.
    expect(statSync(dest).size, 'full export should be multiple MB').toBeGreaterThan(1_000_000);
  });
});
