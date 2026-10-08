import {test, expect} from '@playwright/test';
import {mkdirSync} from 'node:fs';
import {apiCtx, seedWorkspace} from './seed';
import {expectNoHorizontalOverflow} from './helpers';

test.use({viewport:{width:1440,height:900}});
test('saving one skill cannot overwrite the next skill after navigation',async({page})=>{
  const {ctx,base}=await apiCtx();const workspace=await seedWorkspace(ctx,base);
  const names=['r14-original','r14-current'];
  for (const name of names) {
    const response=await ctx.post(`${base}/api/v1/library/skills`,{data:{name,category:'review',description:'Isolated review fixture',body:`# ${name}\n\nOriginal instructions for ${name}.`}});
    expect(response.ok(),await response.text()).toBeTruthy();
  }
  await page.addInitScript((workspace)=>{
    localStorage.setItem('otto_workspace',workspace);
    localStorage.setItem('otto_rail_expanded','0');
    if (!sessionStorage.getItem('r14-theme')) { localStorage.setItem('otto_scheme','light');sessionStorage.setItem('r14-theme','1'); }
  },workspace);
  let release!:()=>void;let started!:()=>void;
  const gate=new Promise<void>(r=>release=r);const requested=new Promise<void>(r=>started=r);
  await page.route('**/library/skills/r14-original/file',async route=>{
    if (route.request().method()!=='PUT') return route.continue();
    const response=await route.fetch();started();await gate;await route.fulfill({response});
  });
  await page.goto('/#/skills-eval');
  await page.getByRole('searchbox',{name:'Filter skills'}).fill('r14-');
  await page.getByTestId('skill-row').filter({hasText:'r14-original'}).click();
  await page.getByRole('tab',{name:'Files',exact:true}).click();
  const editor=page.getByTestId('skill-editor').locator('.cm-content');
  await expect(editor).toContainText('r14-original');
  await editor.fill('# Submitted original skill');
  await page.getByTestId('save-skill').click();await requested;
  await page.getByTestId('skill-row').filter({hasText:'r14-current'}).click();
  await page.getByRole('dialog').getByRole('button',{name:'Discard',exact:true}).click();
  await expect(page.getByTestId('skill-name')).toHaveText('r14-current');
  await expect(editor).toContainText('Original instructions for r14-current');
  const response=page.waitForResponse(r=>r.url().endsWith('/library/skills/r14-original/file')&&r.request().method()==='PUT');
  release();await(await response).finished();
  await page.evaluate(()=>new Promise<void>(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>r()))));
  await expect(page.getByTestId('skill-name')).toHaveText('r14-current');
  await expect(editor).toContainText('Original instructions for r14-current');
  await expect(editor).not.toContainText('Submitted original');
  await expect(page.getByTestId('save-skill')).toBeDisabled();
  const evidence='../docs/reviews/quality-20261008/evidence/R14-skills';mkdirSync(evidence,{recursive:true});
  await expect(page.locator('html')).toHaveAttribute('data-scheme','light');
  await page.screenshot({path:`${evidence}/skills-light.png`,animations:'disabled'});
  await page.evaluate(()=>localStorage.setItem('otto_scheme','dark'));await page.reload();
  await expect(page.getByTestId('skill-name')).toHaveText('r14-current');
  await expect(page.locator('html')).toHaveAttribute('data-scheme','dark');
  await page.screenshot({path:`${evidence}/skills-dark.png`,animations:'disabled'});
  await page.setViewportSize({width:390,height:844});await expectNoHorizontalOverflow(page);
  await page.screenshot({path:`${evidence}/skills-phone.png`,animations:'disabled'});
  await ctx.dispose();
});
