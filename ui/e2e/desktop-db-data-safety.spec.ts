import { test, expect, type Page, type Route } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — data-safety guards, Docker-free (the in-memory "shop" mock
// answers every engine call; the `query` route is overridden per test where a
// specific result shape is needed). The mock connection is `dev` and not
// read-only, i.e. UNGUARDED — the daemon's typed write confirm never fires, so
// these pin the client-side guards:
//   • a destructive statement asks first, and Cancel sends nothing;
//   • a lone BEGIN warns before running and leaves the "own session" notice;
//   • a binary cell (base64 on the wire, `binary: true`) is not editable inline
//     and its viewer shows hex bytes with the length;
//   • a capped result's sort chip says it sorted the loaded rows and "Sort on
//     server" re-runs the statement with ORDER BY;
//   • the full-export dialog defaults to CSV WITH a header row.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
let connId = '';
const CONN = `mock-safety-${Math.random().toString(36).slice(2, 8)}`;

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  connId = await seedMockDbConnection(ctx, base, workspaceId, CONN);
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

/** Answer `query` for statements matching `match` with `body` (registered after
 *  mockDbRoutes, so it wins; everything else falls back to the shop mock). */
async function overrideQuery(page: Page, match: RegExp, body: (stmt: string) => unknown): Promise<void> {
  await page.route(new RegExp(`/connections/${connId}/db/query$`), async (route: Route) => {
    const stmt = String((route.request().postDataJSON() ?? {}).statement ?? '');
    if (!match.test(stmt)) return route.fallback();
    return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body(stmt)) });
  });
}

async function openConn(page: Page): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const c = page.locator('.conn-list .conn-name', { hasText: CONN });
  await expect(c.first()).toBeVisible({ timeout: 30_000 });
  await c.first().click();
  await expect(page.locator('.query-editor')).toBeVisible({ timeout: 20_000 });
}

async function draft(page: Page, stmt: string): Promise<void> {
  const editor = page.locator('.query-editor .qe-edit .cm-content');
  await editor.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText(stmt);
  await expect(editor).toHaveText(stmt);
}

async function clickRun(page: Page): Promise<void> {
  await page.locator('.query-editor .btn.small.primary', { hasText: 'Run' }).first().click();
}

test('a destructive statement asks first; Cancel runs nothing, Run sends it', async ({ page }) => {
  const seen: string[] = [];
  await mockDbRoutes(page, connId, { seen });
  await openConn(page);
  await draft(page, 'DELETE FROM orders');
  await clickRun(page);

  const dialog = page.getByRole('dialog', { name: 'Run a destructive statement?' });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText(CONN);
  await expect(dialog).toContainText('DELETE FROM orders');
  await expect(dialog).toContainText('no WHERE');
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  // A later run proves the queue moved on: by the time it lands, a DELETE sent
  // by the cancelled run would have landed first.
  await draft(page, 'SELECT 1');
  await clickRun(page);
  await expect.poll(() => seen.includes('SELECT 1')).toBe(true);
  expect(seen.some((s) => /DELETE/i.test(s)), 'Cancel must send nothing').toBe(false);
  await draft(page, 'DELETE FROM orders');

  await clickRun(page);
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Run', exact: true }).click();
  await expect.poll(() => seen.filter((s) => /DELETE FROM orders/i.test(s)).length).toBe(1);

  // A WHERE (any WHERE) runs without asking.
  await draft(page, 'DELETE FROM orders WHERE 1=1');
  await clickRun(page);
  await expect.poll(() => seen.filter((s) => /WHERE 1=1/i.test(s)).length).toBe(1);
  await expect(dialog).toHaveCount(0);
});

test('a lone BEGIN warns before running and leaves the own-session notice', async ({ page }) => {
  const seen: string[] = [];
  await mockDbRoutes(page, connId, { seen });
  await openConn(page);
  await draft(page, 'START TRANSACTION');
  await clickRun(page);

  const warn = page.getByRole('dialog', { name: 'The transaction will be rolled back' });
  await expect(warn).toBeVisible();
  await warn.getByRole('button', { name: 'Run anyway', exact: true }).click();
  await expect.poll(() => seen.includes('START TRANSACTION')).toBe(true);
  const notice = page.locator('.tx-notice');
  await expect(notice).toContainText('Each run uses its own session, so a transaction cannot span runs.');
  await notice.getByRole('button', { name: 'Dismiss' }).click();
  await expect(notice).toHaveCount(0);
});

test('a binary cell is read-only and its viewer shows hex bytes', async ({ page }) => {
  await mockDbRoutes(page, connId);
  // 16-byte key and a 4-byte blob, base64 on the wire like the MySQL driver sends.
  const key = Buffer.from('0123456789abcdef0123456789abcdef', 'hex').toString('base64');
  await overrideQuery(page, /FROM\s+orders/i, () => ({
    columns: [
      { name: 'id', type_hint: 'BIGINT' },
      { name: 'notes', type_hint: 'BLOB', binary: true },
    ],
    rows: [[1, key]],
    stats: { duration_ms: 2, row_count: 1 },
    truncated: false,
  }));
  await openConn(page);
  await draft(page, 'SELECT id, notes FROM orders');
  await clickRun(page);
  const cell = page.locator('.query-editor .grid td[data-c="1"]').first();
  await expect(cell).toContainText(key.slice(0, 8));

  // Double-click would start an inline edit on an editable text cell; a binary
  // cell never gets a text box.
  await cell.dblclick();
  await expect(page.locator('.query-editor .grid td input, .query-editor .grid td textarea')).toHaveCount(0);

  const viewer = page.getByRole('dialog', { name: 'Cell value' });
  if (!(await viewer.isVisible())) {
    await cell.click({ button: 'right' });
    await page.getByRole('menuitem', { name: 'Expand value', exact: true }).click();
  }
  await expect(viewer).toBeVisible();
  await expect(viewer).toContainText('Binary · 16 bytes');
  await expect(viewer).toContainText('01 23 45 67 89 ab cd ef 01 23 45 67 89 ab cd ef');
  await expect(viewer.getByRole('button', { name: 'Copy as hex' })).toBeVisible();
  await expect(viewer.getByRole('button', { name: 'Copy as base64' })).toBeVisible();
  await expect(viewer.getByRole('button', { name: 'Edit', exact: true })).toHaveCount(0);
});

test('a capped result says it sorted the loaded rows and can sort on the server', async ({ page }) => {
  const seen: string[] = [];
  await mockDbRoutes(page, connId, { seen });
  await overrideQuery(page, /^SELECT \* FROM orders$/i, () => ({
    columns: [{ name: 'id', type_hint: 'BIGINT' }, { name: 'status', type_hint: 'VARCHAR' }],
    rows: [[1, 'paid'], [2, 'shipped'], [3, 'pending']],
    stats: { duration_ms: 2, row_count: 3 },
    truncated: true,
  }));
  await openConn(page);
  await draft(page, 'SELECT * FROM orders');
  await clickRun(page);
  await expect(page.locator('.query-editor .grid')).toContainText('shipped');

  const head = page.locator('.query-editor .grid thead th', { has: page.locator('.th-name', { hasText: /^status$/ }) });
  await head.click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Sort descending', exact: true }).click();
  await expect(page.locator('.sort-chip')).toContainText('sorted 3 loaded rows');
  await page.getByRole('button', { name: 'Sort on server', exact: true }).click();
  await expect.poll(() => seen.some((s) => /ORDER BY `status` DESC/.test(s))).toBe(true);
});

test('the full-export dialog defaults to CSV with a header row', async ({ page }) => {
  await mockDbRoutes(page, connId);
  await openConn(page);
  await draft(page, 'SELECT * FROM customers LIMIT 5');
  await clickRun(page);
  await expect(page.locator('.query-editor .grid')).toContainText('example.com');
  await page.locator('.grid-toolbar').getByTitle('Copy, download, export all rows, or import a file', { exact: true }).first().click();
  await page.locator('.ctx-item', { hasText: 'Export all rows' }).click();
  const select = page.locator('.exp-form .exp-select');
  await expect(select).toHaveValue('csv_with_names');
  await expect(select.locator('option:checked')).toHaveText('CSV');
  await expect(select.locator('option[value="csv"]')).toHaveText('CSV (no header)');
});
