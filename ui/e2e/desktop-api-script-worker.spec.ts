import {test,expect,type Page} from '@playwright/test';
import {apiCtx,seedWorkspace} from './seed';
import {openApiEditor} from './helpers';

async function setScript(page: Page, code: string, post=false) {
  await page.locator('.builder').getByRole('tab',{name:'Scripts',exact:true}).click();
  const editor=page.locator('.script-block').nth(post?1:0).locator('.cm-content');
  await editor.click(); await editor.press('ControlOrMeta+a'); await editor.fill(code);
}

test('infinite interactive scripts stay cancellable without freezing the page', async ({page},info)=>{
  test.skip(info.project.name!=='desktop-browser','desktop browser only');
  const {ctx,base}=await apiCtx();
  try {
    const ws=await seedWorkspace(ctx,base);
    await page.addInitScript(id=>{localStorage.setItem('otto_workspace',id);localStorage.setItem('otto_firstrun_dismissed','1');},ws);
    let sends=0;
    await page.route('**/api/v1/workspaces/*/api-client/execute',route=>{sends++;return route.fulfill({json:{status:200,status_text:'OK',headers:[],body:'{"ok":true}',duration_ms:1,size_bytes:11}});});
    await openApiEditor(page);await page.getByLabel('Request URL').fill('https://example.test/worker');
    await setScript(page,'while(true){}');
    await page.locator('.builder').getByRole('button',{name:'Send',exact:true}).click();
    const cancel=page.getByTitle('Cancel in-flight request');
    await expect(cancel).toBeVisible();
    // The browser can service a frame and a user action while the Worker loops.
    await page.evaluate(()=>new Promise<void>(resolve=>requestAnimationFrame(()=>resolve())));
    await cancel.click();
    await expect(page.locator('.builder').getByRole('button',{name:'Send',exact:true})).toBeEnabled();
    expect(sends).toBe(0);
    // Without Cancel, the real Worker still terminates at the production deadline.
    await page.locator('.builder').getByRole('button',{name:'Send',exact:true}).click();
    await expect(cancel).toBeVisible();
    await expect(page.locator('.builder').getByRole('button',{name:'Send',exact:true})).toBeEnabled({timeout:10000});
    await expect(page.getByText(/Script timed out after 5 seconds/).first()).toBeVisible();
    expect(sends).toBe(0);
    await setScript(page,"pm.request.method='POST'; pm.environment.set('next','ready')");
    await page.locator('.builder').getByRole('button',{name:'Send',exact:true}).click();
    await expect.poll(()=>sends).toBe(1);
    await expect(page.locator('.builder').getByRole('button',{name:'Send',exact:true})).toBeEnabled();
    await setScript(page,'',false);await setScript(page,'while(true){}',true);
    await page.locator('.builder').getByRole('button',{name:'Send',exact:true}).click();
    await expect.poll(()=>sends).toBe(2);await expect(cancel).toBeVisible();
    await cancel.click();await expect(page.locator('.builder').getByRole('button',{name:'Send',exact:true})).toBeEnabled();
  } finally {await ctx.dispose();}
});
