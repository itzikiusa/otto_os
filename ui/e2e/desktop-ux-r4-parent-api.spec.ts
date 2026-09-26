import {test,expect} from '@playwright/test';
import {apiCtx,seedWorkspace} from './seed';
test.use({serviceWorkers:'block'});
test('typing in the API editor during initial discovery survives its empty response',async({page})=>{
  const {ctx,base}=await apiCtx();const id=await seedWorkspace(ctx,base);await ctx.dispose();
  await page.addInitScript(id=>{localStorage.setItem('otto_workspace',id);localStorage.setItem('otto_firstrun_dismissed','1');},id);
  let release!:()=>void;const held=new Promise<void>(resolve=>release=resolve);let requested=false;
  await page.route(`**/workspaces/${id}/api-client/requests`,async route=>{requested=true;await held;await route.fulfill({json:[]});});
  await page.goto('/#/api');
  await expect.poll(()=>requested).toBe(true);
  const url=page.getByLabel('Request URL',{exact:true});await expect(url).toBeVisible();
  await url.fill('https://fixture.invalid/accepted-draft');
  const loaded=page.waitForResponse(response=>response.url().endsWith(`/workspaces/${id}/api-client/requests`));
  release();await loaded;
  await expect(page.getByText('Loading saved requests…',{exact:true})).toHaveCount(0);
  await expect(url).toHaveValue('https://fixture.invalid/accepted-draft');
  await expect(page.getByText('Create your first request',{exact:true})).toHaveCount(0);
});
