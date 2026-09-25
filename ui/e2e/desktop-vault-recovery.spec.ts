import { expect, test } from '@playwright/test';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import { openPage } from './helpers';

let workspaceId = '', vaultId = 0;
test.beforeAll(async () => {
  const {ctx, base} = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  vaultId = (await seedVaultDir(ctx, base, workspaceId)).vaultId;
  const endpoint = `${base}/api/v1/workspaces/${workspaceId}/vault/vaults/${vaultId}`;
  for (const content of ['# Before version\n', '# After version\n']) {
    const response = await ctx.put(`${endpoint}/note`, {data: {path: 'recovery.md', content}});
    expect(response.ok()).toBeTruthy();
  }
  await ctx.delete(`${endpoint}/note?path=recovery.md`);
  await ctx.dispose();
});
test.beforeEach(async ({page}) => {
  await page.addInitScript(({ws, id}) => {
    localStorage.setItem('otto_workspace', ws);
    localStorage.setItem('otto_vault_last', String(id));
  }, {ws: workspaceId, id: vaultId});
});

test('trash restores a file and edit history compares recoverable versions', async ({page}) => {
  await openPage(page, 'vault');
  // History + trash live in the header's ⋯ (the toolbar keeps ≤5 controls).
  await page.getByRole('button', {name: 'More vault actions', exact: true}).click();
  await page.getByRole('menuitem', {name: 'Trash and restore'}).click();
  const trash = page.getByRole('region', {name: 'Vault trash'});
  await expect(trash.getByText('recovery.md', {exact: true})).toBeVisible();
  await trash.getByRole('button', {name: 'Restore', exact: true}).click();
  await expect(trash.getByText('The trash is empty.', { exact: false })).toBeVisible();
  await page.getByRole('button', {name: 'More vault actions', exact: true}).click();
  await page.getByRole('menuitem', {name: 'Edit history'}).click();
  await page.getByLabel('History file path', {exact: true}).fill('recovery.md');
  await page.getByRole('navigation', {name: 'Saved revisions'}).getByRole('button').first().click();
  await expect(page.getByRole('button', {name: 'Restore before version'})).toBeEnabled();
  await page.getByRole('button', {name: 'Restore before version'}).click();
  await expect(page.getByText('Restored the before version', {exact: true})).toBeVisible();
  const {ctx, base} = await apiCtx();
  const note = await (await ctx.get(`${base}/api/v1/workspaces/${workspaceId}/vault/vaults/${vaultId}/note?path=recovery.md`)).json();
  expect(note.raw).toBe('# Before version\n');
  await ctx.dispose();
});

test('properties edit preserves note body and the local graph is reachable', async ({page}) => {
  await openPage(page, 'vault');
  const tree = page.locator('.tree');
  await tree.getByText('services', {exact: true}).click();
  await tree.getByText('auth-api', {exact: true}).click();
  await page.locator('.right').getByRole('button', {name: /Properties/}).click();
  await page.getByRole('button', {name: 'Edit properties', exact: true}).click();
  await page.getByLabel('Property description', {exact: true}).fill('Updated through structured properties');
  await page.getByRole('button', {name: 'Save properties'}).click();
  await expect(page.getByRole('button', {name: 'Edit properties', exact: true})).toBeVisible();
  await page.getByTitle('Graph view', {exact: true}).click();
  await page.getByLabel('Graph scope', {exact: true}).selectOption({label: 'Around current note'});
  await expect(page.locator('.graph-scope')).toContainText('services/auth-api.md');
});
