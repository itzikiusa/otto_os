import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import { expectNoHorizontalOverflow, expectFullyInViewport, openPage } from './helpers';
import { CANVAS_PAGE_RETIRED } from './storyDesigns';

test.use({ viewport: {width:1280,height:800}, serviceWorkers: 'block' });

let workspaceId = '', vaultId = 0, artifactId = '';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  vaultId = (await seedVaultDir(ctx, base, workspaceId)).vaultId;
  const r = await ctx.post(`${base}/api/v1/design/artifacts`, {data: {workspace_id: workspaceId, title:'Review keyboard draft', format:'html', studio:'frames', content:'<h1>First version</h1>'}});
  expect(r.ok()).toBeTruthy(); artifactId = (await r.json()).artifact.id;
  await ctx.dispose();
});
test.beforeEach(async ({page}) => {
  await page.addInitScript(({w,v}) => { localStorage.setItem('otto_workspace',w); localStorage.setItem('otto_vault_last',String(v)); }, {w:workspaceId,v:vaultId});
});

for (const [theme, scheme, width, rtl] of [['native','light',1440,false],['native','dark',375,false],['warm','light',1024,true],['warm','dark',1440,false],['pro-dark','dark',375,true]] as const) {
  test(`Browser keyboard marking and long content ${theme} ${scheme} ${width}`, async ({page}) => {
    const {ctx,base}=await apiCtx();
    const browserWorkspace=await seedWorkspace(ctx,base);
    await ctx.dispose();
    await page.addInitScript(id=>localStorage.setItem('otto_workspace',id),browserWorkspace);
    await page.setViewportSize({width,height:900});
    await page.addInitScript(({theme,scheme,rtl}) => {localStorage.setItem('otto_theme',theme);localStorage.setItem('otto_scheme',scheme);localStorage.setItem('otto_direction',rtl?'rtl':'ltr');},{theme,scheme,rtl});
    await page.context().route('**/browser/page?url=**', route => route.fulfill({json:{url:'https://example.invalid/keyboard',title:'Keyboard reader',markdown:'A passage for keyboard annotation.\n\n```ts\nconst endpoint = "https://example.invalid/'+ 'long/'.repeat(35)+'";\n```\n\n'+ 'Readable paragraph with useful context. '.repeat(40),engine:'fixture',degraded:false}}));
    await openPage(page,'browser');
    await page.getByPlaceholder('e.g. https://example.com').fill('https://example.invalid/keyboard');
    await page.getByPlaceholder('e.g. https://example.com').press('Enter');
    await expect(page.locator('.reader .page-title')).toHaveText('Keyboard reader');
    await page.getByRole('button',{name:'Mark passage',exact:true}).focus();
    await page.keyboard.press('Enter');
    await page.keyboard.press('Tab');
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('Enter');
    const composer = page.getByRole('textbox',{name:'Note for this mark'});
    await expect(composer).toBeFocused();
    await expect(page.locator('.composer-excerpt')).toHaveText('A passage for keyboard annotation.');
    await composer.fill('Keyboard evidence');
    await page.keyboard.press('Control+Enter');
    await expect(page.locator('.reader p.marked')).toBeVisible();
    await expect(page.getByRole('button',{name:'Mark passage',exact:true})).toBeFocused();
    await expect(page.locator('.reader pre')).toHaveCSS('direction','ltr');
    await expectNoHorizontalOverflow(page);
    await page.screenshot({path:`/tmp/otto-ux-r2-content-browser-${theme}-${scheme}.png`});
  });
}

test('Design Hall tab groups support arrows and Home End', async ({page}) => {
  await openPage(page,'design');
  // The lobby's "Group by" (Recent · Story · Project) is one radiogroup: one
  // tab stop, ←/→ and Home/End move the choice.
  const groupBy = page.getByRole('radiogroup',{name:'Group by'});
  await groupBy.getByRole('radio').first().focus(); await page.keyboard.press('End');
  await expect(groupBy.getByRole('radio').last()).toBeFocused();
  await expect(groupBy.getByRole('radio').last()).toHaveAttribute('aria-checked','true');
  await page.keyboard.press('Home');
  await expect(groupBy.getByRole('radio').first()).toBeFocused();
  await page.keyboard.press('ArrowRight');
  await expect(groupBy.getByRole('radio').nth(1)).toHaveAttribute('aria-checked','true');
  await expect(groupBy.getByRole('radio').first()).toHaveAttribute('tabindex','-1');
  // Team design rules: Proposed · Active.
  await page.goto('/#/design/learned');
  const rules = page.getByRole('tablist',{name:'Team design rules'});
  await rules.getByRole('tab').first().focus(); await page.keyboard.press('End');
  await expect(rules.getByRole('tab').last()).toBeFocused();
  await expect(rules.getByRole('tab').last()).toHaveAttribute('aria-selected','true');
  await page.keyboard.press('Home');
  await expect(rules.getByRole('tab').first()).toBeFocused();
  await expect(rules.getByRole('tab').first()).toHaveAttribute('aria-selected','true');
  await page.goto(`/#/design/a/${artifactId}`);
  const details = page.getByRole('tablist',{name:'Details panel'});
  // Otto · Versions · Links: Otto is first and open by default.
  await details.getByRole('tab').first().focus(); await page.keyboard.press('ArrowRight');
  await expect(page.getByTestId('design-tab-versions')).toBeFocused();
  await expect(page.getByTestId('design-tab-versions')).toHaveAttribute('aria-selected','true');
  await page.getByTestId('design-source-toggle').click();
  await page.locator('.cm-content').first().fill('<h1>Updated keyboard draft</h1>');
  // Autosave makes v2; ↑/↓ move through the version rows.
  await expect(page.getByTestId('design-version-row')).toHaveCount(2);
  const rows = page.getByTestId('design-version-row').locator('button.row-main');
  await rows.first().focus(); await page.keyboard.press('ArrowDown');
  await expect(rows.nth(1)).toBeFocused();
  await page.keyboard.press('Enter');
  await page.getByTestId('design-version-compare').click();
  const compare = page.getByRole('tablist',{name:'Compare mode'});
  await compare.getByRole('tab').first().focus(); await page.keyboard.press('End');
  await expect(compare.getByRole('tab').last()).toBeFocused();
  await expect(compare.getByRole('tab').last()).toHaveAttribute('aria-selected','true');
  await expect(page.getByRole('dialog',{name:'Compare versions'})).toHaveCSS('opacity','1');
  await page.screenshot({path:'/tmp/otto-ux-r2-content-design-compare.png'});
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('design-version-compare')).toBeFocused();
});

test('Vault failed note opens preserve current content with persistent Retry and ignore stale failures', async ({page}) => {
  await openPage(page,'vault');
  const tree=page.locator('.tree');
  await tree.getByText('services',{exact:true}).click();
  await tree.getByText('auth-api',{exact:true}).click();
  await expect(page.locator('.note-view')).toContainText('services/auth-api');
  let failing=true;
  await page.context().route('**/note?path=services%2Forders-api.md', route => failing ? route.fulfill({status:503,json:{code:'upstream',message:'Note storage temporarily unavailable'}}) : route.continue());
  await tree.getByText('orders-api',{exact:true}).click();
  const error=page.getByTestId('note-open-error');
  await expect(error).toContainText('orders-api.md');
  await expect(page.locator('.note-view')).toContainText('services/auth-api');
  expect((await error.boundingBox())!.height).toBeLessThan(100);
  await page.screenshot({path:'/tmp/otto-ux-r2-content-vault-retry.png'});
  failing=false;
  await error.getByRole('button',{name:'Retry',exact:true}).click();
  await expect(page.locator('.note-view')).toContainText('services/orders-api');
  await expect(error).toHaveCount(0);
  let release!:()=>void;
  const held=new Promise<void>(resolve => {release=resolve;});
  let started!:()=>void;
  const requested=new Promise<void>(resolve => {started=resolve;});
  await page.context().route('**/note?path=services%2Fauth-api.md',async route=>{started();await held;await route.fulfill({status:503,json:{code:'upstream',message:'Old request failed'}});});
  await tree.getByText('auth-api',{exact:true}).click(); await requested;
  await tree.getByText('orders-api',{exact:true}).click();
  await expect(page.locator('.note-view')).toContainText('services/orders-api');
  release(); await page.waitForTimeout(250);
  await expect(error).toHaveCount(0);
  await expect(page.locator('.toasts')).not.toContainText('Old request failed');
});

test('Canvas phone no-workspace state opens workspace creation directly',async({page})=>{
  test.skip(true,CANVAS_PAGE_RETIRED);
  await page.setViewportSize({width:375,height:812});
  await page.context().route('**/api/v1/workspaces',route=>route.fulfill({json:[]}));
  await page.addInitScript(()=>localStorage.removeItem('otto_workspace'));
  await openPage(page,'canvas');
  const action=page.getByRole('button',{name:'Choose workspace',exact:true});
  await expectFullyInViewport(page,action);
  await action.click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.keyboard.press('Escape');
  await expect(action).toBeFocused();
  await page.screenshot({path:'/tmp/otto-ux-r2-content-canvas-no-workspace.png'});
});


test('Canvas late scene responses cannot replace the latest selection', async ({page}) => {
  test.skip(true, CANVAS_PAGE_RETIRED);
  const {ctx,base}=await apiCtx();
  const scenes: {id:string; title:string}[]=[];
  for (const title of [`Delayed canvas Alpha ${workspaceId}`,`Latest canvas Beta ${workspaceId}`]) {
    const response=await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/canvas/scenes`,{data:{title,doc:{type:'otto-canvas',version:1,format:'mermaid',source:`flowchart LR\n A[${title}] --> B[Finish]`}}});
    expect(response.ok()).toBeTruthy(); scenes.push(await response.json());
  }
  await ctx.dispose();
  await openPage(page,'canvas');
  await expect(page.locator('.board svg').first()).toBeVisible();
  let release!:()=>void, started!:()=>void;
  const held=new Promise<void>(resolve=>{release=resolve;});
  const requested=new Promise<void>(resolve=>{started=resolve;});
  // The editor reads scenes as `…/canvas/scenes/{id}?files=ref` — match the path, not the full URL.
  await page.context().route(url=>url.pathname.endsWith(`/canvas/scenes/${scenes[0].id}`),async route=>{const response=await route.fetch();started();await held;await route.fulfill({response});});
  await page.locator('.scene-list .row',{hasText:scenes[0].title}).getByRole('button').first().click(); await requested;
  await page.locator('.scene-list .row',{hasText:scenes[1].title}).getByRole('button').first().click();
  await expect(page.locator('.board')).toContainText(scenes[1].title);
  release(); await page.waitForTimeout(500);
  await expect(page.locator('.board')).toContainText(scenes[1].title);
});

test('Design keeps edits typed while a save is in flight dirty',async({page})=>{
  await page.goto(`/#/design/a/${artifactId}`);
  await page.getByTestId('design-source-toggle').click();
  const editor=page.locator('.cm-content').first();
  let release!:()=>void,started!:()=>void;
  const held=new Promise<void>(resolve=>{release=resolve;});
  const requested=new Promise<void>(resolve=>{started=resolve;});
  await page.context().route(`**/design/artifacts/${artifactId}/content`,async route=>{
    if(route.request().method()!=='PUT')return route.continue();
    const response=await route.fetch();started();await held;await route.fulfill({response});
  });
  // The autosave of the first edit is held in flight while more is typed.
  await editor.fill('<h1>Submitted snapshot</h1>');
  await requested;
  const second=page.waitForRequest(r=>r.method()==='PUT'&&r.url().endsWith(`/design/artifacts/${artifactId}/content`)&&(r.postData()??'').includes('Newer unsaved typing'));
  await editor.fill('<h1>Newer unsaved typing</h1>');
  await expect(page.getByTestId('design-dirty')).toBeVisible();
  release();
  // The edits typed meanwhile stay in the editor and are saved next, not dropped.
  await second;
  await expect(editor).toContainText('Newer unsaved typing');
  await expect(page.getByTestId('design-save-state')).toHaveAttribute('data-state','saved');
});

test('Vault retains edits typed during a delayed note open when saving fails',async({page})=>{
  await openPage(page,'vault');
  const tree=page.locator('.tree');
  await tree.getByText('services',{exact:true}).click();
  await tree.getByText('auth-api',{exact:true}).click();
  await page.locator('.mode-btn[title^="Edit"]').click();
  let release!:()=>void,started!:()=>void;
  const held=new Promise<void>(resolve=>{release=resolve;});
  const requested=new Promise<void>(resolve=>{started=resolve;});
  await page.context().route('**/note?path=services%2Forders-api.md',async route=>{
    const response=await route.fetch();started();await held;await route.fulfill({response});
  });
  await page.context().route(`**/vault/vaults/${vaultId}/note`,route=>route.request().method()==='PUT'
    ? route.fulfill({status:503,json:{code:'upstream',message:'Storage temporarily unavailable'}}):route.continue());
  await tree.getByText('orders-api',{exact:true}).click(); await requested;
  const editor=page.locator('.cm-content');
  await editor.fill('# Keep this draft while opening another note');
  release();
  await expect(page.locator('.toasts')).toContainText('Storage temporarily unavailable');
  await expect(editor).toContainText('Keep this draft');
  await expect(page.getByRole('navigation',{name:'Note path'})).toContainText('auth-api');
});
