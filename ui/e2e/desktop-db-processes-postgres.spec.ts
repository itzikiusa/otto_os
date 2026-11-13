import { test, expect, type Page, type Route } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — the Processes panel on a PostgreSQL connection, Docker-free
// (the in-memory mock answers the engine calls; `query` is overridden here):
//   • the pg_stat_activity listing goes out read-only and the panel's own
//     session is not listed;
//   • another role's session reads "Hidden by PostgreSQL", with a notice that
//     names pg_read_all_stats;
//   • Cancel (pg_cancel_backend) and Terminate (pg_terminate_backend) each ask
//     first, naming what happens and the exact statement sent.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
let connId = '';
const CONN = `mock-pgproc-${Math.random().toString(36).slice(2, 8)}`;
const LISTING = /^SELECT pid, usename, datname/;

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  connId = await seedMockDbConnection(ctx, base, workspaceId, CONN, 'postgres');
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

async function overrideQuery(page: Page, match: RegExp, reply: (stmt: string, body: Record<string, unknown>) => unknown): Promise<void> {
  await page.route(new RegExp(`/connections/${connId}/db/query$`), async (route: Route) => {
    const body = (route.request().postDataJSON() ?? {}) as Record<string, unknown>;
    const stmt = String(body.statement ?? '');
    if (!match.test(stmt)) return route.fallback();
    return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(reply(stmt, body)) });
  });
}

test('PostgreSQL Processes: hidden sessions explained, Cancel and Terminate ask first', async ({ page }) => {
  const bodies: Record<string, unknown>[] = [];
  await mockDbRoutes(page, connId, { engine: 'postgres' });
  await overrideQuery(page, /^(SELECT pid, usename|SELECT pg_(cancel|terminate)_backend)/, (stmt, body) => {
    bodies.push(body);
    if (!LISTING.test(stmt)) return { columns: [{ name: 'pg_cancel_backend' }], rows: [[true]], stats: { duration_ms: 1, row_count: 1 } };
    const cols = ['pid', 'usename', 'datname', 'client_addr', 'application_name', 'state', 'wait_event_type', 'wait_event', 'backend_type', 'seconds', 'query'];
    return {
      columns: cols.map((name) => ({ name })),
      rows: [
        [4242, 'app', 'shop', '10.0.0.5/32', 'psql', 'active', 'Lock', 'relation', 'client backend', '12.0', 'UPDATE orders SET slow = 1'],
        [7, 'mock', 'shop', '127.0.0.1/32', 'otto', 'active', null, null, 'client backend', '0.0', `${stmt} LIMIT 501`],
        [99, 'other', 'shop', null, '', null, null, null, null, null, '<insufficient privilege>'],
      ],
      stats: { duration_ms: 1, row_count: 3 },
    };
  });
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const c = page.locator('.conn-list .conn-name', { hasText: CONN });
  await expect(c.first()).toBeVisible({ timeout: 30_000 });
  await c.first().click();
  await expect(page.locator('.query-editor')).toBeVisible({ timeout: 20_000 });
  // Opening folds the list away; the connection's tab carries the same menu.
  await page.locator('.conn-tab', { has: page.locator('.conn-tab-main', { hasText: CONN }) }).first().click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Processes…' }).click();

  const panel = page.getByRole('dialog', { name: `Processes · ${CONN}` });
  const table = panel.getByTestId('db-processes');
  await expect(table).toContainText('UPDATE orders SET slow = 1');
  await expect(table).toContainText('app@10.0.0.5');
  await expect(table).toContainText('Hidden by PostgreSQL');
  await expect(table).not.toContainText('SELECT pid, usename');
  await expect(panel.getByTestId('db-processes-notice')).toContainText('pg_read_all_stats');
  expect(bodies.find((b) => LISTING.test(String(b.statement)))?.read_only).toBe(true);

  await panel.getByRole('button', { name: 'Cancel query 4242' }).click();
  const cancel = page.getByRole('dialog', { name: 'Cancel query' });
  await expect(cancel).toContainText('UPDATE orders SET slow = 1');
  await expect(cancel).toContainText('SELECT pg_cancel_backend(4242)');
  await cancel.getByRole('button', { name: 'Keep running' }).click();
  expect(bodies.some((b) => /pg_cancel_backend/.test(String(b.statement)))).toBe(false);

  await panel.getByRole('button', { name: 'Terminate connection 4242' }).click();
  const term = page.getByRole('dialog', { name: 'Terminate connection' });
  await expect(term).toContainText('pg_terminate_backend(4242)');
  await expect(term).toContainText('rolled back');
  await term.getByRole('button', { name: 'Terminate connection' }).click();
  await expect.poll(() => bodies.some((b) => b.statement === 'SELECT pg_terminate_backend(4242)')).toBe(true);
});
