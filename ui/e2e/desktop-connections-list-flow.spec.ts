import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';
import { pickMenuItem } from './helpers';

// The redesigned Connections flows on the desktop: the list → open → run →
// rows path, and New connection in two steps (paste a URI or pick a kind,
// then that kind's few fields + Test; the rest under More options).
test.use({ serviceWorkers: 'block' });

let workspaceId = '';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  await ctx.dispose();
});
test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((w) => {
    localStorage.setItem('otto_workspace', w as string);
    localStorage.setItem('otto_connhub_filter', 'all');
  }, workspaceId);
});

/** Visible, enabled-or-not interactive controls inside a locator. */
async function controlCount(page: Page, scope: string): Promise<number> {
  return page.locator(scope).evaluate((root) =>
    [...root.querySelectorAll('button, input, select, textarea')].filter((el) => {
      const r = (el as HTMLElement).getBoundingClientRect();
      return r.width > 0 && r.height > 0;
    }).length,
  );
}

test('list → open → run → rows', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const name = `flow-${Date.now()}`;
  const connId = await seedMockDbConnection(ctx, base, workspaceId, name);
  await ctx.dispose();
  await mockDbRoutes(page, connId);
  await page.goto('/#/connections');
  const row = page.locator('.conn-row', { hasText: name });
  await expect(row).toContainText('Not checked');
  await row.click();
  await expect(page).toHaveURL(new RegExp(`#/database/${connId}$`));
  await expect(page.getByTestId('page-header')).toContainText(name);
  const editor = page.locator('.qe-edit .cm-content');
  await editor.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText('SELECT * FROM shop.orders LIMIT 5');
  await page.keyboard.press('ControlOrMeta+Enter');
  await expect(page.locator('.grid tbody tr').first()).toBeVisible();
  // Back on the list, the row now says what the workbench learned.
  await page.getByRole('button', { name: 'All connections', exact: true }).click();
  await expect(page.locator('.conn-row', { hasText: name })).toContainText(/Connected|Can’t connect|Not checked/);
});

test('opening the list reads the rows\' access rights in one request, never one per row', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const tag = `rights-${Date.now()}`;
  const ids: string[] = [];
  for (const n of [1, 2, 3]) ids.push(await seedMockDbConnection(ctx, base, workspaceId, `${tag}-${n}`));
  await ctx.dispose();
  const seen: string[] = [];
  page.on('request', (r) => seen.push(new URL(r.url()).pathname));
  await page.goto('/#/connections');
  await expect(page.locator('.conn-row', { hasText: `${tag}-3` })).toBeVisible();
  await page.waitForTimeout(1500); // ui-guards: allow — proves no late per-row request arrives
  // (A connection another spec adds while the list is open arrives by event
  // and is read on its own; the rows there at open are read together.)
  expect(seen.filter((p) => ids.some((id) => p.includes(`/access/connection/${id}/`))), 'no per-row access read').toEqual([]);
  expect(seen.filter((p) => p === '/api/v1/access/connection/capabilities/batch'), 'one batched access read').toHaveLength(1);
});

test('New connection: pick a kind, then only its fields; the rest folds under More options', async ({ page }) => {
  await page.goto('/#/connections');
  await page.getByRole('button', { name: /New connection/ }).first().click();
  await page.getByRole('menuitem', { name: /^Database…/ }).click();
  const dialog = page.getByRole('dialog');
  // Step 1: a URI field and the kinds — nothing else to fill yet.
  await expect(dialog.getByLabel('Paste a connection URI')).toBeVisible();
  for (const k of ['MySQL', 'PostgreSQL', 'ClickHouse', 'MongoDB', 'Redis']) {
    await expect(dialog.getByRole('button', { name: k, exact: true })).toBeVisible();
  }
  await expect(dialog.getByLabel('Host', { exact: true })).toHaveCount(0);
  expect(await controlCount(page, '[role="dialog"]')).toBeLessThanOrEqual(12);
  // Step 2: MySQL's fields + Test; TLS / SSH tunnel stay folded.
  await dialog.getByRole('button', { name: 'MySQL', exact: true }).click();
  for (const label of ['Name', 'Host', 'User', 'Password']) {
    await expect(dialog.getByLabel(label, { exact: false }).first()).toBeVisible();
  }
  await expect(dialog.getByRole('button', { name: 'Test', exact: true })).toBeVisible();
  await expect(dialog.getByLabel('TLS / SSL')).toHaveCount(0);
  await expect(dialog.getByText('SSH tunnel', { exact: false })).toHaveCount(0);
  expect(await controlCount(page, '[role="dialog"]')).toBeLessThanOrEqual(16);
  await dialog.getByRole('button', { name: /More options/ }).click();
  await expect(dialog.getByLabel('TLS / SSL')).toBeVisible();
  await expect(dialog.getByText('SSH tunnel', { exact: false }).first()).toBeVisible();
});

test('New connection: a pasted URI fills the fields and names the profile', async ({ page }) => {
  await page.goto('/#/connections');
  await page.getByRole('button', { name: /New connection/ }).first().click();
  await page.getByRole('menuitem', { name: /^Database…/ }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByLabel('Paste a connection URI').fill('postgres://app:pw@db.internal:5432/orders');
  await dialog.getByRole('button', { name: 'Continue', exact: true }).click();
  await expect(dialog.getByText('PostgreSQL', { exact: true })).toBeVisible();
  await expect(dialog.getByLabel('Host', { exact: true })).toHaveValue('db.internal');
  await expect(dialog.getByLabel('Name', { exact: true })).toHaveValue('db.internal/orders');
  await expect(dialog.getByLabel(/^User/)).toHaveValue('app');
  await expect(dialog.getByLabel(/^Database/)).toHaveValue('orders');
});

test('Re-pasting a verify-full URI after an allow-invalid-certificates one verifies again (R2-004)', async ({ page }) => {
  const { ctx, base } = await apiCtx(); // one row, so the header carries the New connection menu
  expect((await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/connections`, { data: { name: `tls-paste-${Date.now()}`, kind: 'ssh', params: { host: 'paste.invalid' }, secret: null } })).ok()).toBeTruthy();
  await ctx.dispose();
  await page.goto('/#/connections');
  await page.getByRole('button', { name: /New connection/ }).first().click();
  await page.getByRole('menuitem', { name: /^Database…/ }).click();
  const dialog = page.getByRole('dialog');
  const paste = async (uri: string) => {
    await dialog.getByLabel('Paste a connection URI').fill(uri);
    await dialog.getByRole('button', { name: 'Continue', exact: true }).click();
  };
  await paste('postgres://dev.internal/app?sslmode=require&tlsAllowInvalidCertificates=true');
  await dialog.getByRole('button', { name: /More options/ }).click();
  const verify = dialog.getByLabel('Verify server certificate');
  await expect(verify).not.toBeChecked();
  await dialog.getByRole('button', { name: 'Change', exact: true }).click();
  await paste('postgres://prod.internal/app?sslmode=verify-full');
  await expect(dialog.getByLabel('Host', { exact: true })).toHaveValue('prod.internal');
  if ((await dialog.getByRole('button', { name: /More options/ }).getAttribute('aria-expanded')) !== 'true') await dialog.getByRole('button', { name: /More options/ }).click();
  await expect(dialog.getByLabel('TLS / SSL')).toHaveValue('required');
  await expect(verify).toBeChecked();
});

test('Editing a profile that uses TLS opens More options by itself', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const name = `tls-edit-${Date.now()}`;
  const res = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/connections`, {
    data: { name, kind: 'redis', params: { host: 'fixture.invalid', tls: { mode: 'required', verify: true } }, secret: null },
  });
  expect(res.ok()).toBeTruthy();
  await ctx.dispose();
  await page.goto('/#/connections');
  await pickMenuItem(page, () => page.locator('.conn-row', { hasText: name }).click({ button: 'right' }), 'Edit');
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByLabel('TLS / SSL')).toHaveValue('required');
  await expect(dialog.getByRole('button', { name: /More options/ })).toHaveAttribute('aria-expanded', 'true');
});
