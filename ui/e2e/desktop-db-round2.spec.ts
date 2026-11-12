import { test, expect, type Page, type Route } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — round-2 fixes, Docker-free (the in-memory "shop" mock answers
// every engine call; `query` is overridden per test where a shape is needed):
//   • a Read-only refusal (`read_only_connection:`) is shown as an error and
//     never turns into the typed "type the connection name" confirm;
//   • a TIMESTAMP column's header names the zone it is shown in (UTC);
//   • Hide column / Show all columns from the header menu;
//   • the Processes panel lists read-only, and Kill asks first, naming the
//     query, before it sends KILL QUERY.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
let connId = '';
const CONN = `mock-round2-${Math.random().toString(36).slice(2, 8)}`;

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

/** Answer `query` for statements matching `match` (registered after
 *  mockDbRoutes, so it wins; everything else falls back to the shop mock). */
async function overrideQuery(
  page: Page,
  match: RegExp,
  reply: (stmt: string, body: Record<string, unknown>) => { status?: number; body: unknown },
): Promise<void> {
  await page.route(new RegExp(`/connections/${connId}/db/query$`), async (route: Route) => {
    const body = (route.request().postDataJSON() ?? {}) as Record<string, unknown>;
    const stmt = String(body.statement ?? '');
    if (!match.test(stmt)) return route.fallback();
    const r = reply(stmt, body);
    return route.fulfill({ status: r.status ?? 200, contentType: 'application/json', body: JSON.stringify(r.body) });
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

const TS_RESULT = {
  columns: [
    { name: 'id', type_hint: 'BIGINT' },
    { name: 'status', type_hint: 'VARCHAR' },
    { name: 'created_at', type_hint: 'TIMESTAMP' },
  ],
  rows: [
    [1, 'paid', '2026-11-12 08:00:00'],
    [2, 'shipped', '2026-11-12 09:30:00'],
  ],
  stats: { duration_ms: 2, row_count: 2 },
};

test('a Read-only refusal is shown as an error, with no typed-name confirm', async ({ page }) => {
  const seen: string[] = [];
  await mockDbRoutes(page, connId, { seen });
  await overrideQuery(page, /^UPDATE /i, () => ({
    status: 403,
    body: {
      code: 'forbidden',
      message:
        "read_only_connection: read-only connection 'x' refuses writes and schema changes; turn off Read-only in the connection's settings to allow them",
    },
  }));
  await openConn(page);
  await draft(page, 'UPDATE orders SET status = 1 WHERE id = 1');
  await clickRun(page);
  await expect(page.getByText(/refuses writes and schema changes/).first()).toBeVisible();
  await expect(page.getByRole('dialog', { name: /Confirm (write|production write)/ })).toHaveCount(0);
});

test('a TIMESTAMP column header names the zone it is shown in', async ({ page }) => {
  await mockDbRoutes(page, connId);
  await overrideQuery(page, /^SELECT \* FROM orders$/i, () => ({ body: TS_RESULT }));
  await openConn(page);
  await draft(page, 'SELECT * FROM orders');
  await clickRun(page);
  const head = page.locator('.query-editor .grid thead th', { has: page.locator('.th-name', { hasText: /^created_at$/ }) });
  await expect(head.getByTestId('db-col-zone')).toHaveText('· UTC');
  await expect(head).toHaveAttribute('title', /Shown in UTC .*does not convert/);
  // A non-temporal column carries no marker.
  const status = page.locator('.query-editor .grid thead th', { has: page.locator('.th-name', { hasText: /^status$/ }) });
  await expect(status.getByTestId('db-col-zone')).toHaveCount(0);
});

test('Hide column removes it from the grid and Show all brings it back', async ({ page }) => {
  await mockDbRoutes(page, connId);
  await overrideQuery(page, /^SELECT \* FROM orders$/i, () => ({ body: TS_RESULT }));
  await openConn(page);
  await draft(page, 'SELECT * FROM orders');
  await clickRun(page);
  const names = page.locator('.query-editor .grid thead .th-name');
  await expect(names).toHaveText(['id', 'status', 'created_at']);

  await names.filter({ hasText: /^status$/ }).click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Hide column', exact: true }).click();
  await expect(names).toHaveText(['id', 'created_at']);
  await expect(page.locator('.query-editor .grid')).not.toContainText('shipped');

  await names.filter({ hasText: /^id$/ }).click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Show all columns (1 hidden)' }).click();
  await expect(names).toHaveText(['id', 'status', 'created_at']);
});

test('Processes lists read-only and Kill asks first, naming the query', async ({ page }) => {
  const seen: string[] = [];
  const bodies: Record<string, unknown>[] = [];
  await mockDbRoutes(page, connId, { seen });
  await overrideQuery(page, /^(SHOW FULL PROCESSLIST|KILL QUERY )/i, (stmt, body) => {
    bodies.push(body);
    if (/^KILL/i.test(stmt)) return { body: { columns: [], rows: [], stats: { duration_ms: 1, row_count: 0 } } };
    return {
      body: {
        columns: ['Id', 'User', 'Host', 'db', 'Command', 'Time', 'State', 'Info'].map((name) => ({ name })),
        rows: [[42, 'app', '10.0.0.5:5123', 'shop', 'Query', 17, 'Sending data', 'SELECT * FROM orders WHERE slow = 1']],
        stats: { duration_ms: 1, row_count: 1 },
      },
    };
  });
  await openConn(page);
  await page.locator('.conn-list .conn-name', { hasText: CONN }).first().click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Processes…' }).click();

  const panel = page.getByRole('dialog', { name: `Processes · ${CONN}` });
  await expect(panel.getByTestId('db-processes')).toContainText('SELECT * FROM orders WHERE slow = 1');
  const listing = bodies.find((b) => b.statement === 'SHOW FULL PROCESSLIST');
  expect(listing?.read_only).toBe(true);

  await panel.getByRole('button', { name: 'Kill query 42' }).click();
  const confirm = page.getByRole('dialog', { name: 'Kill query' });
  await expect(confirm).toContainText('SELECT * FROM orders WHERE slow = 1');
  await expect(confirm).toContainText('KILL QUERY 42');
  // Cancel sends nothing.
  await confirm.getByRole('button', { name: 'Cancel' }).click();
  expect(bodies.some((b) => /^KILL/i.test(String(b.statement)))).toBe(false);

  await panel.getByRole('button', { name: 'Kill query 42' }).click();
  await page.getByRole('dialog', { name: 'Kill query' }).getByRole('button', { name: 'Kill query' }).click();
  await expect.poll(() => bodies.some((b) => b.statement === 'KILL QUERY 42')).toBe(true);
});

test('History → All connections lists rows labelled with their connection', async ({ page }) => {
  await mockDbRoutes(page, connId);
  const entry = { id: 'h1', connection_id: connId, statement: 'SELECT 42 AS answer', ok: true, duration_ms: 1, row_count: 1, created_at: new Date().toISOString() };
  await page.route(new RegExp(`/connections/${connId}/db/history\\?`), (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([entry]) }),
  );
  await openConn(page);
  await page.getByRole('tab', { name: 'History' }).click();
  await page.getByLabel('Whose history').selectOption({ label: 'All connections' });
  const row = page.getByTestId('db-history-all-row').filter({ hasText: 'SELECT 42 AS answer' });
  await expect(row).toContainText(CONN);
  // The connection filter lists the connections that answered.
  await expect(page.getByLabel('Show history of').locator('option', { hasText: CONN })).toHaveCount(1);
});
