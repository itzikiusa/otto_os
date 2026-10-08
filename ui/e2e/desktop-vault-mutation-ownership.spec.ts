import {test, expect} from '@playwright/test';
import {mkdirSync} from 'node:fs';
import {apiCtx, seedWorkspace, seedVaultDir} from './seed';
import {expectNoHorizontalOverflow} from './helpers';

test.use({viewport: {width: 1440, height: 900}});

test('delayed OKF toggle stays with its original vault after switching', async ({page}) => {
  const {ctx, base} = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  const a = await seedVaultDir(ctx, base, workspace);
  const b = await seedVaultDir(ctx, base, workspace);
  const endpoint = `${base}/api/v1/workspaces/${workspace}/vault/vaults`;
  await ctx.patch(`${endpoint}/${a.vaultId}`, {data: {name: 'Original vault'}});
  await ctx.patch(`${endpoint}/${b.vaultId}`, {data: {name: 'Current vault'}});
  await ctx.dispose();
  await page.addInitScript(({workspace, id}) => {
    localStorage.setItem('otto_workspace', workspace);
    if (!sessionStorage.getItem('r11-ready')) {
      localStorage.setItem('otto_vault_last', String(id));
      localStorage.setItem('otto_scheme', 'light');
      sessionStorage.setItem('r11-ready', 'yes');
    }
  }, {workspace, id: a.vaultId});
  let release!: () => void;
  let started!: () => void;
  const gate = new Promise<void>(resolve => {release = resolve;});
  const requested = new Promise<void>(resolve => {started = resolve;});
  await page.route(`**/vault/vaults/${a.vaultId}`, async route => {
    if (route.request().method() !== 'PATCH') return route.continue();
    const response = await route.fetch();
    started();
    await gate;
    await route.fulfill({response});
  });
  await page.goto('/#/vault');
  await page.getByTitle('Original vault — switch vault', {exact: true}).click();
  await page.getByRole('menuitemcheckbox', {name: 'OKF mode (validation + templates)', exact: true}).click();
  await requested;
  await page.getByTitle('Original vault — switch vault', {exact: true}).click();
  await page.getByRole('menuitemcheckbox', {name: 'Current vault', exact: true}).click();
  await expect(page.getByTitle('Current vault — switch vault', {exact: true})).toBeVisible();
  const response = page.waitForResponse(r => r.url().endsWith(`/vault/vaults/${a.vaultId}`) && r.request().method() === 'PATCH');
  release(); await (await response).finished();
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  await expect(page.getByTitle('Current vault — switch vault', {exact: true})).toBeVisible();
  await page.locator('.tree').getByText('services', {exact: true}).click();
  await page.locator('.tree').getByText('auth-api', {exact: true}).click();
  await expect(page.getByRole('navigation', {name: 'Note path'})).toContainText('auth-api');
  const evidence = '../docs/reviews/quality-20261008/evidence';
  mkdirSync(evidence, {recursive: true});
  await expect(page.locator("html")).toHaveAttribute("data-scheme", "light");
  await page.screenshot({path: `${evidence}/R11-vault-light.png`});
  await page.evaluate(() => { localStorage.setItem('otto_scheme', 'dark'); });
  // Scheme is applied by the shell listener on a reload; keep the active vault.
  await page.reload();
  await expect(page.getByTitle('Current vault — switch vault', {exact: true})).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-scheme", "dark");
  await page.screenshot({path: `${evidence}/R11-vault-dark.png`});
  await page.setViewportSize({width: 390, height: 844});
  await expectNoHorizontalOverflow(page);
  await page.screenshot({path: `${evidence}/R11-vault-phone.png`});
});
