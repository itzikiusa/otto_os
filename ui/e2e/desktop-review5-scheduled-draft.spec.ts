import {test, expect} from '@playwright/test';
import {apiCtx, seedWorkspace} from './seed';

test.use({serviceWorkers: 'block'});
test('scheduled task deep link and save retry preserve newer edits and the leave decision', async ({page}) => {
  const {ctx, base} = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  const created = await ctx.post(`${base}/api/v1/workspaces/${workspace}/scheduled-tasks`, {data: {
    name: 'Scoped scheduled draft', prompt: 'Original prompt', schedule: {cadence: 'interval', every_min: 60},
    destination: {type: 'none'}, enabled: false,
  }});
  expect(created.ok()).toBe(true);
  const task = await created.json();
  await ctx.dispose();
  await page.addInitScript(id => localStorage.setItem('otto_workspace', id), workspace);
  const writes: {name: string; prompt: string}[] = [];
  let release!: () => void;
  const gate = new Promise<void>(resolve => {release = resolve;});
  await page.route(`**/api/v1/scheduled-tasks/${task.id}`, async route => {
    if (route.request().method() !== 'PATCH') return route.continue();
    writes.push(route.request().postDataJSON());
    if (writes.length === 1) return route.fulfill({status: 503, json: {code: 'unavailable', message: 'Synthetic save failure'}});
    if (writes.length === 2) await gate;
    return route.continue();
  });
  await page.goto(`/#/scheduled-tasks/${task.id}`);
  const row = page.locator(`[data-task-id="${task.id}"]`);
  await expect(row.getByRole('button', {name: 'Runs', exact: true})).toHaveAttribute('aria-expanded', 'true');
  await row.getByRole('button', {name: 'More actions for Scoped scheduled draft'}).click();
  await page.getByRole('menuitem', {name: 'Edit…', exact: true}).click();
  const name = page.getByRole('textbox', {name: 'Name', exact: true});
  const prompt = page.getByRole('textbox', {name: 'Prompt (the agent’s instructions)', exact: true});
  await name.fill('Submitted scheduled name'); await prompt.fill('Submitted scheduled prompt');
  await page.getByRole('button', {name: 'Save changes', exact: true}).click();
  await expect(page.locator('.sched [role="alert"]')).toContainText('Synthetic save failure');
  await expect(prompt).toHaveValue('Submitted scheduled prompt');
  await page.getByRole('button', {name: 'Save changes', exact: true}).click();
  await expect.poll(() => writes.length).toBe(2);
  await name.fill('Newer scheduled name'); await prompt.fill('Newer scheduled prompt');
  release();
  await expect(page.getByRole('button', {name: 'Save changes', exact: true})).toBeEnabled();
  await expect(prompt).toHaveValue('Newer scheduled prompt');
  await page.getByRole('button', {name: 'Back to Scheduled Tasks', exact: true}).click();
  await page.getByRole('dialog').getByRole('button', {name: 'Cancel', exact: true}).click();
  await expect(prompt).toHaveValue('Newer scheduled prompt');
  await page.getByRole('button', {name: 'Save changes', exact: true}).click();
  await expect(row).toContainText('Newer scheduled name');
  expect(writes.map(({name, prompt}) => ({name, prompt}))).toEqual([
    {name: 'Submitted scheduled name', prompt: 'Submitted scheduled prompt'},
    {name: 'Submitted scheduled name', prompt: 'Submitted scheduled prompt'},
    {name: 'Newer scheduled name', prompt: 'Newer scheduled prompt'},
  ]);
  await expect(page).toHaveURL(new RegExp(`/scheduled-tasks/${task.id}$`));
});
