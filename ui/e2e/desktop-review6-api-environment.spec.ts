import {test, expect} from '@playwright/test';
import {apiCtx, seedWorkspace} from './seed';
import {openApiEditor} from './helpers';

test.use({serviceWorkers: 'block'});
for (const revisit of [false, true]) test(`environment pending Save reconciles ${revisit ? 'clean A-B-A selection' : 'newer values and a second secret rename'}`, async ({page}) => {
  const {ctx, base} = await apiCtx();
  try {
    const workspace = await seedWorkspace(ctx, base);
    const api = `${base}/api/v1/workspaces/${workspace}/api-client`;
    const a = await ctx.post(`${api}/environments`, {data: {name: 'Environment A', variables: {base_url: 'original'}, secret_keys: ['token'], secret_values: {token: 'isolated-test-secret'}}});
    expect(a.ok()).toBe(true); const env = await a.json();
    expect((await ctx.post(`${api}/environments`, {data: {name: 'Environment B', variables: {base_url: 'other'}}})).ok()).toBe(true);
    await page.addInitScript(id => localStorage.setItem('otto_workspace', id), workspace);
    await openApiEditor(page);
    await page.getByRole('button', {name: /^Environment:/}).click();
    await page.getByRole('menuitem', {name: 'Manage environments…', exact: true}).click();
    await page.locator('.env-pick').filter({hasText: 'Environment A'}).click();
    const editor = page.getByRole('region', {name: 'Variables in Environment A'});
    const value = editor.getByRole('textbox', {name: 'Value of base_url', exact: true});
    const secretKey = editor.locator('.var-row').filter({has: page.getByLabel('Value of token', {exact: true})}).getByLabel('Variable name', {exact: true});
    await value.fill('submitted'); await secretKey.fill('first_token');
    let release!: () => void;
    const gate = new Promise<void>(resolve => {release = resolve;});
    const writes: {secret_renames: Record<string, string>}[] = [];
    await page.route(`**/api-client/environments/${env.id}`, async route => {
      if (route.request().method() !== 'PATCH') return route.continue();
      writes.push(route.request().postDataJSON());
      if (writes.length === 1) await gate;
      return route.continue();
    });
    await editor.getByRole('button', {name: 'Save changes', exact: true}).click();
    await expect.poll(() => writes.length).toBe(1);
    if (revisit) {
      await page.locator('.env-pick').filter({hasText: 'Environment B'}).click();
      await page.getByRole('dialog').getByRole('button', {name: 'Discard', exact: true}).click();
      await page.locator('.env-pick').filter({hasText: 'Environment A'}).click();
    } else {
      await value.fill('newer');
      await editor.locator('.var-row').filter({has: page.getByLabel('Value of first_token', {exact: true})}).getByLabel('Variable name', {exact: true}).fill('second_token');
    }
    release();
    const save = editor.getByRole('button', {name: 'Save changes', exact: true});
    await expect(save).toBeVisible();
    await expect(value).toHaveValue(revisit ? 'submitted' : 'newer');
    if (revisit) {
      await expect(save).toBeDisabled();
      await expect(editor.getByLabel('Value of first_token', {exact: true})).toHaveValue('');
    } else {
      await expect(save).toBeEnabled(); await save.click();
      await expect(save).toBeDisabled();
      expect(writes[1].secret_renames).toEqual({first_token: 'second_token'});
    }
    const saved = (await (await ctx.get(`${api}/environments`)).json()).find((row: {id: string}) => row.id === env.id);
    expect(saved.variables.base_url).toBe(revisit ? 'submitted' : 'newer');
    expect(saved.secret_keys).toEqual([revisit ? 'first_token' : 'second_token']);
    expect(JSON.stringify(saved)).not.toContain('isolated-test-secret');
  } finally {await ctx.dispose();}
});
