import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — the query error panel (error-normalize.ts + ErrorPanel.svelte).
//
// Docker-free: the in-memory "shop" mock answers every engine call, and the
// `query` route is overridden to fail the way the daemon does — a 502 Problem
// whose message is the cleaned engine text plus tagged trailer lines
// (`ERRNO:` / `SQLSTATE:` / `SUGGEST:` / `POSITION:`). Pins:
//   • a short headline + code chip instead of the raw text;
//   • a "did you mean" chip that edits the statement in the editor;
//   • the caret excerpt for a reported position;
//   • the raw text hidden behind "Show full error" (with Copy).
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
let connId = '';
let pgConnId = '';
const CONN = `mock-err-${Math.random().toString(36).slice(2, 8)}`;
const PG_CONN = `mock-err-pg-${Math.random().toString(36).slice(2, 8)}`;

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  connId = await seedMockDbConnection(ctx, base, workspaceId, CONN);
  pgConnId = await seedMockDbConnection(ctx, base, workspaceId, PG_CONN, 'postgres');
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

/** Make the connection's `query` route fail with `message` (registered after
 *  mockDbRoutes, so it wins for that one op). */
async function failQueries(page: Page, id: string, message: string): Promise<void> {
  await page.route(new RegExp(`/connections/${id}/db/query$`), (route) =>
    route.fulfill({
      status: 502,
      contentType: 'application/json',
      body: JSON.stringify({ code: 'upstream', message }),
    }),
  );
}

async function openConn(page: Page, name: string): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const c = page.locator('.conn-list .conn-name', { hasText: name });
  await expect(c.first()).toBeVisible({ timeout: 30_000 });
  await c.first().click();
  await expect(page.locator('.query-editor')).toBeVisible({ timeout: 20_000 });
}

async function typeAndRun(page: Page, stmt: string): Promise<void> {
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText(stmt);
  await page.waitForTimeout(250);
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
}

test('MySQL unknown column: headline, code chip, did-you-mean chip edits the editor, raw behind a toggle', async ({ page }) => {
  await mockDbRoutes(page, connId);
  await failQueries(
    page,
    connId,
    "upstream: Unknown column 'nme' in 'field list'\nERRNO: 1054\nSQLSTATE: 42S22\nSUGGEST: name",
  );
  await openConn(page, CONN);
  await typeAndRun(page, 'SELECT nme FROM customers');

  const panel = page.locator('.err-panel');
  await expect(panel).toBeVisible({ timeout: 15_000 });
  await expect(panel.locator('.err-title')).toHaveText('Unknown column `nme`.');
  await expect(panel.locator('.err-code')).toHaveText('Error 1054 · 42S22');

  // The raw text (with its trailer lines) is collapsed until asked for.
  const raw = panel.locator('.err-msg');
  await expect(raw).toBeHidden();
  await panel.locator('.err-raw summary').click();
  await expect(raw).toBeVisible();
  await expect(raw).toContainText('ERRNO: 1054');
  await expect(panel.locator('.err-copy')).toBeVisible();

  // The chip replaces the unknown name in the editor.
  await panel.locator('.err-chip', { hasText: 'name' }).click();
  await expect(page.locator('.qe-edit .cm-content')).toHaveText('SELECT name FROM customers');
});

test('Postgres position becomes a caret excerpt under the reported column', async ({ page }) => {
  await mockDbRoutes(page, pgConnId, { engine: 'postgres' });
  await failQueries(
    page,
    pgConnId,
    'upstream: column "nme" does not exist\nSQLSTATE: 42703\nPOSITION: 8',
  );
  await openConn(page, PG_CONN);
  await typeAndRun(page, 'SELECT nme FROM customers');

  const panel = page.locator('.err-panel');
  await expect(panel.locator('.err-title')).toHaveText('Unknown column `nme`.', { timeout: 15_000 });
  await expect(panel.locator('.err-code')).toHaveText('42703');
  await expect(panel.locator('.err-excerpt-label')).toHaveText('line 1, col 8');
  // Caret sits under column 8 (7 spaces, then ^).
  await expect(panel.locator('.err-excerpt-code')).toHaveText('SELECT nme FROM customers\n       ^');
});
