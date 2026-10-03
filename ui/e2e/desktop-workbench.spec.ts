import { test, expect, type Page, type Route } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';
import { expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Workbench — scratch files with full history. One flow end to end on the
// throwaway daemon: New file → type → Format → Preview → history (two ⌘S
// checkpoints, restore the older one) → Send to → Database — Run on…, which
// opens the DB Explorer's sheet prefilled with the script and its placeholder
// sweep (brands 1,2,3,4). The DB engine calls are answered by `page.route`
// (db-mock); nothing is ever run.
// ─────────────────────────────────────────────────────────────────────────────

const suffix = Math.random().toString(36).slice(2, 8);
const CONN = `wb-db-${suffix}`;
let workspaceId = '';
let connId = '';
let base = '';
let token = '';

const MINIFIED = '{"b":1,"a":[1,2]}';
const SQL = 'SELECT * FROM orders WHERE brand_id = :brand';

test.beforeAll(async () => {
  const c = await apiCtx();
  base = c.base;
  token = c.token;
  workspaceId = await seedWorkspace(c.ctx, c.base);
  connId = await seedMockDbConnection(c.ctx, c.base, workspaceId, CONN);
  await c.ctx.dispose();
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'Desktop flow');
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.removeItem(`otto_db_open:${id}`);
  }, workspaceId);
});

/** Authenticated JSON call against the throwaway daemon. */
async function daemon<T>(page: Page, method: string, path: string, data?: unknown): Promise<T> {
  const r = await page.request.fetch(`${base}/api/v1${path}`, {
    method,
    headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    data: data === undefined ? undefined : JSON.stringify(data),
  });
  expect(r.ok(), `${method} ${path} → ${r.status()} ${await r.text()}`).toBeTruthy();
  return (r.status() === 204 ? undefined : await r.json()) as T;
}

interface DocRow {
  id: string;
  name: string;
}

async function openWorkbench(page: Page): Promise<void> {
  await page.goto('/#/workbench');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
}

/** The confirmer dialog's primary (or danger) button. */
async function confirmTopDialog(page: Page): Promise<void> {
  const dlg = page.locator('.sheet[role="dialog"]').last();
  await expect(dlg).toBeVisible();
  await dlg.locator('button.btn.primary, button.btn.danger-solid').last().click();
}

/** A side panel (Placeholders / History) may sit behind a tab or toggle. */
async function revealPlaceholders(page: Page): Promise<void> {
  const panel = page.getByTestId('wb-placeholders');
  if (await panel.isVisible()) return;
  const tab = page.getByRole('tab', { name: /Placeholders/i }).or(page.getByRole('button', { name: /Placeholders/i }));
  await tab.first().click();
  await expect(panel).toBeVisible();
}

test('create → type → format → preview → history restore', async ({ page }) => {
  await openWorkbench(page);

  // New file (an optional name prompt is answered with a .json name).
  await page.getByTestId('wb-new').first().click();
  const prompt = page.locator('.sheet[role="dialog"] input.cf-input');
  if (await prompt.waitFor({ state: 'visible', timeout: 2_000 }).then(() => true, () => false)) {
    await prompt.fill('scratch.json');
    await prompt.press('Enter');
  }
  const editor = page.getByTestId('wb-editor').locator('.cm-content');
  await expect(editor).toBeVisible();
  await editor.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText(MINIFIED);

  // ⌘S: a checkpoint revision holding the minified text.
  await page.keyboard.press('ControlOrMeta+S');
  const listDocs = () => daemon<DocRow[]>(page, 'GET', `/workspaces/${workspaceId}/workbench/docs`);
  await expect.poll(async () => (await listDocs()).length).toBeGreaterThan(0);
  const docId = (await listDocs())[0]!.id;
  await expect
    .poll(async () => (await daemon<{ content: string }>(page, 'GET', `/workspaces/${workspaceId}/workbench/docs/${docId}`)).content)
    .toBe(MINIFIED);
  const revsBefore = (await daemon<unknown[]>(page, 'GET', `/workspaces/${workspaceId}/workbench/docs/${docId}/revisions`)).length;

  // Format: pretty JSON, then another checkpoint.
  await page.getByTestId('wb-format').first().click();
  await expect(editor).toContainText('"b": 1');
  await editor.click();
  await page.keyboard.press('ControlOrMeta+S');
  await expect
    .poll(async () => (await daemon<unknown[]>(page, 'GET', `/workspaces/${workspaceId}/workbench/docs/${docId}/revisions`)).length)
    .toBeGreaterThan(revsBefore);

  // Preview (JSON tree) beside the editor.
  await page.getByTestId('wb-preview-toggle').first().click();
  await expect(page.getByTestId('wb-preview')).toBeVisible();
  await expectNoHorizontalOverflow(page);

  // History: newest first — restore the previous (minified) checkpoint.
  await page.getByTestId('wb-history-toggle').first().click();
  const rows = page.getByTestId('wb-rev-row');
  await expect.poll(() => rows.count()).toBeGreaterThan(1);
  const older = rows.nth(1);
  await older.click();
  const inRow = older.getByTestId('wb-rev-restore');
  if (await inRow.count()) await inRow.click();
  else await page.getByTestId('wb-rev-restore').first().click();
  await confirmTopDialog(page);
  await expect(editor).not.toContainText('"b": 1');
  await expect(editor).toContainText(MINIFIED);
  await expect
    .poll(async () => (await daemon<{ content: string }>(page, 'GET', `/workspaces/${workspaceId}/workbench/docs/${docId}`)).content)
    .toBe(MINIFIED);
});

test('Send to → Database — Run on… opens prefilled with the brand sweep', async ({ page }) => {
  await mockDbRoutes(page, connId);
  await page.route('**/api/v1/db/multi-runs', (route: Route) =>
    route.request().method() === 'GET' ? route.fulfill({ json: [] }) : route.abort(),
  );
  await page.route('**/api/v1/db/multi-run/plan', (route: Route) => route.abort());
  const doc = await daemon<DocRow>(page, 'POST', `/workspaces/${workspaceId}/workbench/docs`, {
    name: `brands-${suffix}.sql`,
    language: 'sql',
    content: SQL,
  });

  await openWorkbench(page);
  await page.getByTestId('wb-file-row').filter({ hasText: doc.name }).first().click();
  await expect(page.getByTestId('wb-editor').locator('.cm-content')).toContainText('brand_id');

  await revealPlaceholders(page);
  const panel = page.getByTestId('wb-placeholders');
  await expect(panel).toContainText('brand');
  await panel.getByLabel('Value for brand').fill('1,2,3,4');
  await expect(panel).toContainText('4 values');

  await page.getByTestId('wb-sendto').click();
  await page.getByRole('menuitem', { name: /Database — Run on/ }).click();

  // The only connection is picked; the sheet opens on setup — nothing runs.
  const sheet = page.getByTestId('multi-run');
  await expect(sheet).toBeVisible({ timeout: 20_000 });
  await expect(sheet.getByLabel('Values for brand')).toHaveValue('1,2,3,4');
  await expect(page.locator('.query-editor .qe-edit .cm-content')).toContainText(SQL);
  await expect(page).toHaveURL(/#\/database/);
});
