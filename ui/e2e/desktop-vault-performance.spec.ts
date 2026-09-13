import { expect, test } from '@playwright/test';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import { openPage } from './helpers';

// Run only with root's isolated performance setup and current test binary.
// Files belong exclusively to seedVaultDir's newly created temporary fixture.
let workspaceId = '', vaultId = 0, vaultDir = '';
test.beforeEach(async ({ page }) => {
  const { ctx, base } = await apiCtx();
  try {
    workspaceId = await seedWorkspace(ctx, base);
    const seeded = await seedVaultDir(ctx, base, workspaceId);
    vaultId = seeded.vaultId; vaultDir = seeded.dir;
    mkdirSync(join(vaultDir, 'services', 'nested'));
    writeFileSync(join(vaultDir, 'services', 'nested', 'visible.md'), '# Nested visible\n');
    expect((await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/vault/vaults/${vaultId}/rescan`, { data: {} })).ok()).toBeTruthy();
  } finally { await ctx.dispose(); }
  await page.addInitScript(({ ws, id }) => {
    localStorage.setItem('otto_workspace', ws);
    localStorage.setItem('otto_vault_last', String(id));
    localStorage.setItem(`otto_vault_last:${ws}`, String(id));
    localStorage.setItem('otto_rail_expanded', '0');
  }, { ws: workspaceId, id: vaultId });
});

test('rescan preserves expanded descendants and refreshes collapsed branches on reopening', async ({ page }) => {
  await openPage(page, 'vault');
  const tree = page.locator('.tree');
  await tree.getByText('services', { exact: true }).click();
  await tree.getByText('nested', { exact: true }).click();
  await expect(tree.getByText('visible', { exact: true })).toBeVisible();
  await tree.getByText('runbooks', { exact: true }).click();
  await expect(tree.getByText('deploy', { exact: true })).toBeVisible();
  await tree.getByText('runbooks', { exact: true }).click();
  await expect(tree.getByText('deploy', { exact: true })).toBeHidden();
  const directories: string[] = [];
  page.on('request', request => {
    const url = new URL(request.url());
    if (url.pathname.endsWith(`/vault/vaults/${vaultId}/dir`)) directories.push(url.searchParams.get('path') ?? '');
  });
  writeFileSync(join(vaultDir, 'services', 'added.md'), '# Added while open\n');
  writeFileSync(join(vaultDir, 'runbooks', 'later.md'), '# Added while collapsed\n');
  await page.getByTitle('Rescan vault', { exact: true }).click();
  await expect(tree.getByText('added', { exact: true })).toBeVisible();
  await expect(tree.getByText('visible', { exact: true })).toBeVisible();
  expect(directories).toContain('services');
  expect(directories).toContain('services/nested');
  expect(directories).not.toContain('runbooks');
  await expect(tree.getByText('later', { exact: true })).toBeHidden();
  await tree.getByText('runbooks', { exact: true }).click();
  await expect(tree.getByText('later', { exact: true })).toBeVisible();
  expect(directories).toContain('runbooks');
});

test('external rescan preserves a dirty editor while refreshing its visible branch', async ({ page }) => {
  await openPage(page, 'vault');
  const tree = page.locator('.tree');
  await tree.getByText('services', { exact: true }).click();
  await tree.getByText('auth-api', { exact: true }).click();
  // Keep the local draft dirty through the real client's conflict handling.
  await page.route(`**/vault/vaults/${vaultId}/note`, async route => {
    if (route.request().method() !== 'PUT') return route.continue();
    await route.fulfill({ status: 409, contentType: 'application/json', body: JSON.stringify({ code: 'conflict', message: 'Fixture changed on disk' }) });
  });
  await page.locator('.mode-btn[title^="Edit"]').click();
  const editor = page.locator('.cm-content');
  await editor.fill('# Unsaved local draft\nLocal changes must remain.');
  await expect(page.locator('.conflict[role="alert"]')).toContainText('changed on disk');
  writeFileSync(join(vaultDir, 'services', 'auth-api.md'), '# External disk version\n');
  writeFileSync(join(vaultDir, 'services', 'fresh.md'), '# Fresh tree entry\n');
  await page.getByTitle('Rescan vault', { exact: true }).click();
  await expect(tree.getByText('fresh', { exact: true })).toBeVisible();
  await expect(editor).toContainText('Unsaved local draft');
  await expect(editor).not.toContainText('External disk version');
  expect(readFileSync(join(vaultDir, 'services', 'auth-api.md'), 'utf8')).toBe('# External disk version\n');
});

test('oversized source and hash stay exact with explicit metadata-only status', async () => {
  const source = `[[auth-api]] #oldtag\n${'x'.repeat(4 * 1024 * 1024)}`;
  const file = join(vaultDir, 'large.md');
  writeFileSync(file, source);
  const { ctx, base } = await apiCtx();
  const endpoint = `${base}/api/v1/workspaces/${workspaceId}/vault/vaults/${vaultId}`;
  try {
    expect((await ctx.post(`${endpoint}/rescan`, { data: {} })).ok()).toBeTruthy();
    const response = await ctx.get(`${endpoint}/note?path=large.md`);
    expect(response.ok()).toBeTruthy();
    const note = await response.json();
    expect(note.meta.content_index_status).toBe('size_limited');
    expect(note.meta.tags).toEqual([]);
    expect(note.outgoing).toEqual([]);
    expect(note.meta.hash).toBe(createHash('sha256').update(source).digest('hex'));
    expect(note.raw).toBe(source);
    expect(readFileSync(file, 'utf8')).toBe(source);
  } finally { await ctx.dispose(); }
});
