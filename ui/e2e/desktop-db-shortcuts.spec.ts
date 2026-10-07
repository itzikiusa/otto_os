import { test, expect, type Page } from '@playwright/test';
import { execFileSync, spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
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
const MYSQL_CONTAINER = process.env.OTTO_E2E_MYSQL_CONTAINER ?? 'otto-dbv-mysql';
const MYSQL_ARGS = ['exec', '-i', MYSQL_CONTAINER, 'mysql', '-uotto', '-pottopw',
  '--batch', '--skip-column-names', '--unbuffered', 'shopdb'];

function mysql(sql: string): string {
  return execFileSync('docker', MYSQL_ARGS, { input: sql, encoding: 'utf8', timeout: 10_000,
    stdio: ['pipe', 'pipe', 'pipe'] }).trim();
}

/** A private table lock makes an ordinary SELECT stay in flight under the
 * current enforced policy, which deliberately refuses the SLEEP function. */
async function withLockedTable(run: (table: string) => Promise<void>): Promise<void> {
  const table = `e2e_cancel_${randomUUID().replaceAll('-', '')}`;
  const locker = spawn('docker', MYSQL_ARGS, { stdio: ['pipe', 'pipe', 'pipe'] });
  let output = '';
  let errors = '';
  locker.stdout.on('data', (chunk) => { output += String(chunk); });
  locker.stderr.on('data', (chunk) => { errors += String(chunk); });
  const exited = new Promise<number | null>((resolve, reject) => {
    locker.once('error', reject);
    locker.once('close', resolve);
  });
  // Bound a lock even if the worker is interrupted. Only this fixture's unique
  // table is locked; other specs' MySQL tables remain available.
  locker.stdin.write(`SET SESSION wait_timeout = 30; CREATE TABLE ${table} (id INT); ` +
    `LOCK TABLES ${table} WRITE; SELECT 'locked';\n`);
  try {
    await expect.poll(() => output, { message: 'Fixture acquired its private table lock' }).toContain('locked');
    await run(table);
  } finally {
    locker.stdin.end(`UNLOCK TABLES; DROP TABLE IF EXISTS ${table};\n`);
    const timer = setTimeout(() => locker.kill('SIGKILL'), 10_000);
    try {
      expect(await exited, errors).toBe(0);
    } finally {
      clearTimeout(timer);
    }
  }
}

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
  const conn = page.locator(`.conn-row[data-connection-id="${mysqlConn}"] .conn-name`);
  await expect(conn).toBeVisible({ timeout: 30_000 });
  await conn.click();
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

test('running status shows during a slow query; Esc cancels it', async ({ page }) => {
  test.skip(!mysqlConn, 'mysql docker not reachable');
  await openMysql(page);
  await withLockedTable(async (table) => {
    const sql = `SELECT COUNT(*) FROM shopdb.${table}`;
    await typeStatement(page, sql);
    await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();

    // Observe the actual engine waiting on our lock before cancelling: a brief
    // loading flash followed by a rejected query must not satisfy this test.
    const waitingQueries = () => mysql(`SELECT COUNT(*) FROM information_schema.processlist ` +
      `WHERE INFO LIKE '${sql}%' AND STATE LIKE '%lock%';`);
    await expect.poll(waitingQueries, { timeout: 10_000 }).toBe('1');
    const running = page.getByRole('status', { name: 'Running query', exact: true });
    await expect(running).toBeVisible();
    await expect(page.locator('.rg-overlay-text')).toContainText(/Running…/);

    // The lock remains held until all assertions finish, so only cancellation
    // can end this run. Check the server's native cancellation outcome too.
    const cancelled = page.waitForResponse((response) =>
      response.url().endsWith(`/connections/${mysqlConn}/db/cancel`) &&
      response.request().method() === 'POST');
    await page.locator('.qe-edit .cm-content').click();
    await page.keyboard.press('Escape');
    const response = await cancelled;
    expect(response.ok()).toBeTruthy();
    expect(await response.json()).toMatchObject({ status: 'cancelled' });
    await expect(running).toHaveCount(0, { timeout: 8_000 });
    await expect.poll(waitingQueries).toBe('0');
    await expect(page.locator('.btn.small.primary', { hasText: 'Run' }).first()).toBeVisible({
      timeout: 8_000,
    });
  });
});
