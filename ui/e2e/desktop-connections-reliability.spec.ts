import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

test('editing retains advanced TLS configuration and duplication omits credentials', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  const name = `retained-${Date.now()}`;
  const response = await ctx.post(`${base}/api/v1/workspaces/${ws}/connections`, { data: {
    name, kind: 'redis', params: { host: 'fixture.invalid', secure: true, custom_option: 'preserved', database: '0' }, secret: 'isolated-fixture-secret',
  } });
  expect(response.ok()).toBeTruthy();
  const original = await response.json();
  await openPage(page, 'connections');
  await page.locator('.conn-row', { hasText: name }).click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  await page.getByLabel('Name', { exact: true }).fill(`${name}-edited`);
  await page.getByRole('button', { name: 'Save changes', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Save changes', exact: true })).toBeHidden();
  const list = await ctx.get(`${base}/api/v1/workspaces/${ws}/connections`);
  const saved = (await list.json()).find((c: { id: string }) => c.id === original.id);
  expect(saved.params.custom_option).toBe('preserved');
  expect(saved.params.tls.mode).toBe('required');
  expect(saved.secret_ref).toBe(original.secret_ref);
  // Saving a DB profile opens its workbench; return to the unified picker.
  await page.getByRole('tab', { name: 'Connections', exact: true }).click();
  await page.locator('.conn-row', { hasText: `${name}-edited` }).click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Duplicate without password', exact: true }).click();
  await expect(page.getByLabel('Name', { exact: true })).toHaveValue(`${name}-edited (copy)`);
  const copies = await ctx.get(`${base}/api/v1/workspaces/${ws}/connections`);
  const copy = (await copies.json()).find((c: { name: string }) => c.name === `${name}-edited (copy)`);
  expect(copy.secret_ref).toBeNull();
  expect(copy.params.custom_option).toBe('preserved');
  await ctx.dispose();
});

test('Mongo URI credentials are normalized by the server and SFTP errors wait for Retry', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  const mongo = await ctx.post(`${base}/api/v1/workspaces/${ws}/connections`, { data: {
    name: `mongo-secret-${Date.now()}`, kind: 'mongodb', params: { conn_string: 'mongodb://fixture:p%40ss@one.invalid,two.invalid/db?tls=true' },
  } });
  expect(mongo.ok()).toBeTruthy();
  const normalized = await mongo.json();
  expect(normalized.params.conn_string).toBe('mongodb://fixture:{secret}@one.invalid,two.invalid/db?tls=true');
  expect(normalized.secret_ref).toBeTruthy();
  const name = `sftp-retry-${Date.now()}`;
  const ssh = await ctx.post(`${base}/api/v1/workspaces/${ws}/connections`, { data: { name, kind: 'ssh', params: { host: 'fixture.invalid' } } });
  expect(ssh.ok()).toBeTruthy();
  const { id } = await ssh.json();
  let attempts = 0;
  // Intercept only this fixture listing: no SSH host is contacted.
  await page.route(`**/connections/${id}/sftp/list**`, async route => {
    attempts++;
    await route.fulfill({ status: 502, contentType: 'application/json', body: JSON.stringify({ code: 'upstream', message: 'fixture authentication failed' }) });
  });
  await openPage(page, 'connections');
  await page.locator('.conn-row', { hasText: name }).click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Browse files (SFTP)', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Retry', exact: true })).toBeVisible();
  await page.waitForTimeout(700);
  expect(attempts).toBe(1);
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect.poll(() => attempts).toBe(2);
  await ctx.dispose();
});

test('SSH settings can be tested before saving and a failure says what to fix', async ({ page }) => {
  await openPage(page, 'connections');
  await page.getByRole('button', { name: 'New connection', exact: true }).first().click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('button', { name: 'SSH', exact: true }).click();
  await dialog.getByLabel('Name', { exact: true }).fill(`ssh-test-${Date.now()}`);
  // Real daemon round trip (no ssh is spawned): a profile without a host has
  // nothing to probe and must not report "Connected".
  await dialog.getByRole('button', { name: 'Test', exact: true }).click();
  const panel = dialog.locator('.test-panel');
  await expect(panel).toContainText('Test failed');
  await expect(panel).toContainText('nothing to test');
  // A recognised failure shows the full message and the fix hint (fulfilled
  // fixture — no SSH host is contacted).
  await page.route('**/connections/unsaved/test', route => route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({
      ok: false, latency_ms: 812, warn_argv: false,
      message: 'deploy@fixture.invalid: Permission denied (publickey).',
      hint: 'the server rejected the login. Check the user name and the identity file, or load the key into ssh-agent (`ssh-add`).',
    }),
  }));
  await dialog.getByLabel('Host', { exact: true }).fill('fixture.invalid');
  await expect(panel).toBeHidden(); // editing clears the stale result
  await dialog.getByRole('button', { name: 'Test', exact: true }).click();
  await expect(panel).toContainText('Permission denied (publickey).');
  await expect(panel).toContainText('What to check:');
  await expect(panel).toContainText('ssh-add');
  await expect(dialog.getByText('Test failed — see details above')).toBeVisible();
});
