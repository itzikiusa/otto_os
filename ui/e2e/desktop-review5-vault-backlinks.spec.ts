import {test, expect} from '@playwright/test';
import {apiCtx, seedWorkspace, seedVaultDir} from './seed';

test.use({serviceWorkers: 'block', viewport: {width: 1440, height: 1000}});
test('Vault backlinks failure stays distinct from empty and Retry restores the selected note links', async ({page}) => {
  const {ctx, base} = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  const {vaultId} = await seedVaultDir(ctx, base, workspace);
  await ctx.dispose();
  await page.addInitScript(({workspace, vaultId}) => {
    localStorage.setItem('otto_workspace', workspace);
    localStorage.setItem('otto_vault_last', String(vaultId));
  }, {workspace, vaultId});
  let fail = true;
  const queries: string[] = [];
  await page.route('**/backlinks?*', route => {
    const path = new URL(route.request().url()).searchParams.get('path') ?? '';
    if (path !== 'services/auth-api.md') return route.continue();
    queries.push(path);
    return fail ? route.fulfill({status: 503, json: {code: 'unavailable', message: 'Synthetic backlinks unavailable'}})
      : route.fulfill({json: [{path: 'services/orders-api.md', title: 'Recovery backlink', context: 'Orders depends on authentication', kind: 'wiki'}]});
  });
  await page.goto('/#/vault');
  const tree = page.locator('.tree');
  await tree.getByText('services', {exact: true}).click();
  await tree.getByText('auth-api', {exact: true}).click();
  const right = page.locator('.right');
  await expect(right.getByTestId('load-error')).toContainText('Synthetic backlinks unavailable');
  await expect(right.getByText('No linked mentions', {exact: true})).toHaveCount(0);
  fail = false;
  await right.getByRole('button', {name: 'Retry', exact: true}).click();
  await expect(right.getByRole('button', {name: /Recovery backlink/})).toBeVisible();
  await expect(right.getByTestId('load-error')).toHaveCount(0);
  await expect(page.getByRole('navigation', {name: 'Note path'})).toContainText('auth-api');
  expect(queries.length).toBeGreaterThanOrEqual(2);
});
