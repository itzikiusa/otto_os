import {test, expect, type Page} from '@playwright/test';
import {apiCtx, seedWorkspace} from './seed';
import {openPage} from './helpers';

let workspaceA='', workspaceB='';
test.beforeAll(async()=>{
  const {ctx,base}=await apiCtx();
  workspaceA=await seedWorkspace(ctx,base);workspaceB=await seedWorkspace(ctx,base);
  await ctx.dispose();
});
test.beforeEach(async({page},info)=>{
  test.skip(info.project.name!=='desktop-browser','persistent desktop page lifecycle');
  await page.addInitScript(id=>localStorage.setItem('otto_workspace',id),workspaceA);
});
async function select(page:Page,id:string) {
  await page.evaluate(async next=>{
    const path='/src/lib/stores/workspace.svelte.ts';
    void (await import(path)).ws.select(next);
  },id);
}
async function current(page:Page) {
  return page.evaluate(async()=>{const path='/src/lib/stores/workspace.svelte.ts';return (await import(path)).ws.currentId;});
}

test('goal draft Keep stays in A; workspace Discard closes the persistent form and does not resurrect',async({page})=>{
  // Only the provider response is stubbed. The real page, component, router,
  // workspace selection, modal and parent creation state remain mounted.
  await page.route('**/api/v1/workspaces/*/goal-loops/define',route=>route.fulfill({json:{
    definition:{title:'Workspace A draft',summary:'A scoped goal',acceptance_criteria:[{id:'c1',text:'A criterion',verify:'A evidence',verify_kind:'agent',verify_cmd:null}]},
    suggested_limits:{max_iterations:5,max_runtime_secs:1800,per_phase_timeout_secs:600},
    suggested_config:{executors:[{name:'Executor',provider:'claude',model:'',prompt_extra:''}]},
  }}));
  await openPage(page,'loops');
  await page.getByRole('button',{name:/New goal loop|Define your first goal/}).first().click();
  await page.locator('#gl-seed').fill('Keep all A fields');
  await page.getByPlaceholder('/absolute/path/to/repo').fill('/tmp/workspace-A');
  await page.getByRole('button',{name:'Define with AI'}).click();
  await page.getByPlaceholder('Criterion description').fill('Edited A criterion');
  await page.getByLabel('Max minutes',{exact:true}).fill('120');
  await select(page,workspaceB);
  const dialog=page.getByRole('dialog');
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button',{name:/^(Cancel|Keep editing)$/}).click();
  await expect.poll(()=>current(page)).toBe(workspaceA);
  await expect(page.locator('#gl-seed')).toHaveValue('Keep all A fields');
  await expect(page.getByPlaceholder('Criterion description')).toHaveValue('Edited A criterion');
  await expect(page.getByLabel('Max minutes',{exact:true})).toHaveValue('120');
  await expect(page.getByPlaceholder('/absolute/path/to/repo')).toHaveValue('/tmp/workspace-A');
  await select(page,workspaceB);
  await dialog.getByRole('button',{name:'Discard',exact:true}).click();
  await expect.poll(()=>current(page)).toBe(workspaceB);
  await expect(page.locator('#gl-seed')).toHaveCount(0);
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await select(page,workspaceA);
  await expect.poll(()=>current(page)).toBe(workspaceA);
  await expect(page.locator('#gl-seed')).toHaveCount(0);
  await page.getByRole('button',{name:/New goal loop|Define your first goal/}).first().click();
  await expect(page.locator('#gl-seed')).toHaveValue('');
  // Even a clean creation form must settle its parent's creating state.
  await select(page,workspaceB);
  await expect.poll(()=>current(page)).toBe(workspaceB);
  await expect(page.locator('#gl-seed')).toHaveCount(0);
  await expect(page.getByRole('dialog')).toHaveCount(0);
});
