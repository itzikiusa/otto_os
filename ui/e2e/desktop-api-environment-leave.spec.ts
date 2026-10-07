import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openApiEditor } from './helpers';

test.use({ serviceWorkers: 'block' });

async function setup(page: Page) {
  const { ctx, base } = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  const otherWorkspace = await seedWorkspace(ctx, base);
  const api = `${base}/api/v1/workspaces/${workspace}/api-client`;
  const response = await ctx.post(`${api}/environments`, { data: { name: 'Environment A', variables: { base_url: 'original' }, secret_keys: ['token'] } });
  expect(response.ok()).toBe(true);
  const env = await response.json();
  expect((await ctx.post(`${api}/environments`, { data: { name: 'Environment B', variables: { base_url: 'other' } } })).ok()).toBe(true);
  await page.addInitScript(id => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspace);
  await openApiEditor(page);
  await page.getByRole('button', { name: /^Environment:/ }).click();
  await page.getByRole('menuitem', { name: 'Manage environments…', exact: true }).click();
  await page.locator('.env-pick').filter({ hasText: 'Environment A' }).click();
  return { ctx, api, env, workspace, otherWorkspace };
}

const value = (page: Page) => page.getByRole('textbox', { name: 'Value of base_url', exact: true });
const home = (page: Page) => page.getByRole('navigation', { name: /^(Navigator|Modules)$/ }).getByRole('button', { name: 'Home', exact: true });

test('environment draft guards request tabs, close, module, workspace, selection and creation without persisting secrets', async ({ page }) => {
  const fixture = await setup(page);
  try {
    await value(page).fill('unsaved');
    await page.getByLabel('Value of token', { exact: true }).fill('environment-draft-secret');
    const exits = [
      () => page.locator('.req-tab:not(.special) .req-tab-main').first().click(),
      () => page.getByRole('button', { name: 'Close environments', exact: true }).click(),
      () => home(page).click(),
      () => page.locator('.env-pick').filter({ hasText: 'Environment B' }).click(),
      () => page.getByRole('button', { name: 'New environment', exact: true }).click(),
      async () => {
        await page.getByRole('button', { name: /^Environment:/ }).click();
        await page.getByRole('menuitem', { name: 'New environment…', exact: true }).click();
      },
      // Exercise the real workspace-selection path, which asks router guards
      // before changing the persistence context, without reloading the page.
      () => page.evaluate(async id => {
        const path = '/src/lib/stores/workspace.svelte.ts';
        const { ws } = await import(/* @vite-ignore */ path);
        void ws.select(id);
      }, fixture.otherWorkspace),
    ];
    for (const leave of exits) {
      await leave();
      const dialog = page.getByRole('dialog');
      await expect(dialog).toContainText('unsaved changes');
      await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
      await expect(value(page)).toHaveValue('unsaved');
      await expect(page.getByLabel('Value of token', { exact: true })).toHaveValue('environment-draft-secret');
      await expect(page).toHaveURL(/#\/api$/);
    }
    const persisted = await page.evaluate(() => JSON.stringify({ ...localStorage, ...sessionStorage }));
    expect(persisted).not.toContain('environment-draft-secret');
    await home(page).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Discard', exact: true }).click();
    await expect(page).toHaveURL(/#\/home$/);
    const saved = (await (await fixture.ctx.get(`${fixture.api}/environments`)).json()).find((row: { id: string }) => row.id === fixture.env.id);
    expect(saved.variables.base_url).toBe('original');
  } finally { await fixture.ctx.dispose(); }
});

test('environment leave Save waits, retains failures and newer edits, then leaves only after saving the current draft', async ({ page }) => {
  const fixture = await setup(page);
  let release = () => {};
  try {
    let fail = true;
    let attempts = 0;
    let blocked = Promise.resolve();
    await page.route(`**/api-client/environments/${fixture.env.id}`, async route => {
      if (route.request().method() !== 'PATCH') return route.continue();
      attempts++;
      await blocked;
      return fail ? route.fulfill({ status: 503, json: { error: { code: 'unavailable', message: 'Save fixture unavailable' } } }) : route.continue();
    });
    await value(page).fill('submitted');
    await home(page).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Save', exact: true }).click();
    await expect.poll(() => attempts).toBe(1);
    await expect(page.getByRole('button', { name: 'Save changes', exact: true })).toBeEnabled();
    await expect(value(page)).toHaveValue('submitted');
    await expect(page).toHaveURL(/#\/api$/);

    fail = false;
    blocked = new Promise<void>(resolve => { release = resolve; });
    await home(page).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Save', exact: true }).click();
    await expect.poll(() => attempts).toBe(2);
    await expect(page).toHaveURL(/#\/api$/);
    await expect(value(page)).toHaveValue('submitted');
    await value(page).fill('newer');
    release();
    await expect(page.getByRole('button', { name: 'Save changes', exact: true })).toBeEnabled();
    await expect(value(page)).toHaveValue('newer');
    await expect(page).toHaveURL(/#\/api$/);
    await home(page).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Save', exact: true }).click();
    await expect(page).toHaveURL(/#\/home$/);
    expect(attempts).toBe(3);
    const saved = (await (await fixture.ctx.get(`${fixture.api}/environments`)).json()).find((row: { id: string }) => row.id === fixture.env.id);
    expect(saved.variables.base_url).toBe('newer');
  } finally { release(); await fixture.ctx.dispose(); }
});
