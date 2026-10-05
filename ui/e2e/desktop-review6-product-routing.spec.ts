import {test, expect} from '@playwright/test';
import {apiCtx, seedWorkspace} from './seed';

test.use({serviceWorkers: 'block'});
test('Product explicit story URL wins initial selection and canceled route changes keep the unsaved draft', async ({page}) => {
  const {ctx, base} = await apiCtx();
  try {
    const workspace = await seedWorkspace(ctx, base);
    const stories: string[] = [];
    for (const title of ['Routed draft A', 'Latest draft B']) {
      const response = await ctx.post(`${base}/api/v1/workspaces/${workspace}/product/drafts`, {data: {title}});
      expect(response.ok()).toBe(true);
      stories.push((await response.json()).story.id);
    }
    await page.addInitScript(id => localStorage.setItem('otto_workspace', id), workspace);
    await page.goto(`/#/product/${stories[0]}`);
    await expect(page.locator('#draft-title')).toHaveValue('Routed draft A');
    const body = page.getByRole('textbox', {name: 'Body (Markdown)', exact: true});
    await body.fill('Unsaved A must survive refused navigation');
    const navigate = () => page.evaluate(async id => {
      const path = '/src/lib/router.svelte.ts';
      (await import(path)).router.go(`product/${id}`);
    }, stories[1]);
    await navigate();
    await page.getByRole('dialog').getByRole('button', {name: 'Keep editing', exact: true}).click();
    await expect(page).toHaveURL(new RegExp(`/product/${stories[0]}$`));
    await expect(body).toHaveValue('Unsaved A must survive refused navigation');
    await expect(page.locator('#draft-title')).toHaveValue('Routed draft A');
    await navigate();
    await page.getByRole('dialog').getByRole('button', {name: 'Discard', exact: true}).click();
    await expect(page.locator('#draft-title')).toHaveValue('Latest draft B');
    await expect(page).toHaveURL(new RegExp(`/product/${stories[1]}$`));
    await expect(body).toHaveValue('');
    const original = await (await ctx.get(`${base}/api/v1/product/stories/${stories[0]}`)).json();
    expect(original.source.body_md ?? '').toBe('');
  } finally {await ctx.dispose();}
});
