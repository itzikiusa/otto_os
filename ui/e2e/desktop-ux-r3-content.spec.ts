import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import { openPage, expectNoHorizontalOverflow, expectFullyInViewport } from './helpers';
test.use({ viewport:{width:1280,height:900}, serviceWorkers:'block' });
let workspaceId='', vaultId=0;
test.beforeAll(async()=>{const {ctx,base}=await apiCtx();workspaceId=await seedWorkspace(ctx,base);vaultId=(await seedVaultDir(ctx,base,workspaceId)).vaultId;await ctx.dispose();});
test.beforeEach(async({page})=>{await page.addInitScript(({w,v})=>{localStorage.setItem('otto_workspace',w);localStorage.setItem('otto_vault_last',String(v));},{w:workspaceId,v:vaultId});});
async function scenes(){const {ctx,base}=await apiCtx();const rows=[];const suffix=`${Date.now()}-${Math.random().toString(36).slice(2,7)}`;for(const title of [`R3 Alpha ${suffix}`,`R3 Beta ${suffix}`]) {const r=await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/canvas/scenes`,{data:{title,doc:{type:'otto-canvas',version:1,format:'mermaid',source:`flowchart LR\n A[${title}] --> B[Finish]`}}});expect(r.ok()).toBeTruthy();rows.push(await r.json());}await ctx.dispose();return rows;}
async function selectScene(page:Page,title:string){await page.locator('.scene-list .row',{hasText:title}).getByRole('button').first().click();await expect(page.locator('.board')).toContainText(title);}
test('Canvas preserves a failed pending draft across scene switches',async({page})=>{
 const [a,b]=await scenes();await openPage(page,'canvas');await selectScene(page,a.title);
 await page.getByTitle('Edit the Mermaid source',{exact:true}).click();
 let fail=true;const writes:string[]=[];
 await page.context().route(`**/canvas/scenes/${a.id}`,async route=>{if(route.request().method()!=='PUT')return route.continue();writes.push(route.request().postDataJSON().doc.source);if(fail)return route.fulfill({status:503,json:{code:'upstream',message:'Synthetic save unavailable'}});return route.continue();});
 await page.locator('.cm-content').fill('flowchart LR\n A[Retain pending draft] --> B[Recover]');
 await selectScene(page,b.title);await expect.poll(()=>writes.length).toBeGreaterThan(0);
 await page.locator('.scene-list .row',{hasText:a.title}).getByRole('button').first().click();
 await expect(page.locator('.board')).toContainText('Retain pending draft');
 await expect(page.locator('.save-error')).toContainText('Synthetic save unavailable');
 fail=false;
 await page.getByRole('button',{name:'Retry save',exact:true}).click();
 await expect(page.locator('.save-error')).toHaveCount(0);
 await page.getByTitle('Edit the Mermaid source',{exact:true}).click();
 await page.locator('.cm-content').fill('flowchart LR\n A[Recovered draft] --> B[Saved]');
 await expect.poll(()=>writes.at(-1)).toContain('Recovered draft');
});
test('Reader new page clears old pending mark and nested links leave one tab stop',async({page})=>{
 const {ctx,base}=await apiCtx();const w=await seedWorkspace(ctx,base);await ctx.dispose();await page.addInitScript(id=>localStorage.setItem('otto_workspace',id),w);
 await page.context().route('**/browser/page?url=**',route=>{const second=route.request().url().includes('second');return route.fulfill({json:{url:`https://example.invalid/${second?'second':'first'}`,title:second?'Second reader':'First reader',markdown:'Paragraph with [nested link](https://example.invalid/nested).\n\n> Nested **quote** with [link](https://example.invalid/quote).',engine:'fixture',degraded:false}});});
 await openPage(page,'browser');const url=page.getByPlaceholder('Enter URL');await url.fill('https://example.invalid/first');await url.press('Enter');await expect(page.locator('.reader h1')).toHaveText('First reader');
 await page.getByRole('button',{name:'Mark passage',exact:true}).click();
 await expect(page.locator('.reader article a').first()).toHaveAttribute('tabindex','-1');
 await page.locator('.reader article p').first().click();await page.getByRole('textbox',{name:'Note for this mark'}).fill('Old source draft');
 await url.fill('https://example.invalid/second');await url.press('Enter');await expect(page.locator('.reader h1')).toHaveText('Second reader');
 await expect(page.getByRole('textbox',{name:'Note for this mark'})).toHaveCount(0);
 await expect(page.getByRole('button',{name:'Mark passage',exact:true})).toBeVisible();
});
for(const [theme,scheme,width,rtl] of [['native','light',1440,false],['native','dark',375,false],['warm','light',1024,true],['warm','dark',375,false],['pro-dark','dark',1440,true]] as const){
 test(`Loaded Vault Design Snip ${theme} ${scheme} ${width}`,async({page})=>{
  await page.setViewportSize({width,height:900});await page.addInitScript(({theme,scheme,rtl})=>{localStorage.setItem('otto_theme',theme);localStorage.setItem('otto_scheme',scheme);localStorage.setItem('otto_direction',rtl?'rtl':'ltr');},{theme,scheme,rtl});
  await openPage(page,'vault');
  const tree=page.locator('.tree');await tree.getByText('services',{exact:true}).click();await tree.getByText('auth-api',{exact:true}).click();
  await expect(page.locator('.note-view')).toContainText('services/auth-api');await expectNoHorizontalOverflow(page);await page.screenshot({path:`/tmp/otto-ux-r3-content-vault-${theme}-${scheme}.png`});
  const {ctx,base}=await apiCtx();const r=await ctx.post(`${base}/api/v1/design/artifacts`,{data:{workspace_id:workspaceId,title:'Synthetic loaded review card',format:'html',studio:'frames',content:'<html><body style="font:16px system-ui;padding:24px"><h1>Launch checklist</h1><p>Review the draft before publishing.</p><button>Preview draft</button></body></html>'}});expect(r.ok()).toBeTruthy();const id=(await r.json()).artifact.id;
  await page.goto(`/#/design/a/${id}`);await expect(page.getByTestId('design-source-toggle')).toBeVisible();
  if(width<640) await page.getByRole('group',{name:'Device frame'}).getByRole('button',{name:'iPhone',exact:true}).click();
  await page.getByTestId('design-source-toggle').click();await page.locator('.cm-content').first().fill('<h1>Updated on this screen</h1><p>Draft remains editable at every width.</p>');await page.getByTestId('design-save').click();await expect(page.getByTestId('design-version-chip')).toHaveCount(2);await page.getByTestId('design-source-toggle').click();
  await expectNoHorizontalOverflow(page);await page.screenshot({path:`/tmp/otto-ux-r3-content-design-${theme}-${scheme}.png`});
  const png=await page.evaluate(()=>{const c=document.createElement('canvas');c.width=640;c.height=480;const x=c.getContext('2d')!;x.fillStyle='#eeeeee';x.fillRect(0,0,640,480);x.fillStyle='#222222';x.font='24px sans-serif';x.fillText('Synthetic screenshot for annotation',35,70);return c.toDataURL().split(',')[1];});
  const snip=await ctx.post(`${base}/api/v1/snips`,{data:{data_b64:png,filename:'synthetic-review.png'}});expect(snip.ok()).toBeTruthy();const sid=(await snip.json()).id;await ctx.dispose();
  await page.goto(`/#/snip/${sid}`);const drawing=page.locator('.snip-canvas');await expect(drawing).toBeVisible();await expectNoHorizontalOverflow(page);
  if(width<640) expect((await page.getByRole('button',{name:'Text (T)',exact:true}).boundingBox())!.height).toBeGreaterThanOrEqual(36);
  await page.getByRole('button',{name:'Text (T)',exact:true}).click();const bounds=(await drawing.boundingBox())!;await drawing.click({position:{x:bounds.width-8,y:bounds.height-8}});
  await expectFullyInViewport(page,page.locator('.snip-textentry'),'Text editor at image edge');
  await page.screenshot({path:`/tmp/otto-ux-r3-content-snip-${theme}-${scheme}-entry.png`});
  await page.locator('.snip-textentry').fill('Readable annotation');await page.keyboard.press('Control+Enter');await expect(page.locator('.snip-editor')).toHaveAttribute('data-count','1');
  await expect(page.locator('.snip-copied')).toHaveText('Copied');await page.screenshot({path:`/tmp/otto-ux-r3-content-snip-${theme}-${scheme}.png`});
 });
}
test('Canvas queues in-flight saves and keeps both scene drafts',async({page})=>{
 const [a,b]=await scenes();await openPage(page,'canvas');await selectScene(page,a.title);await page.getByTitle('Edit the Mermaid source',{exact:true}).click();
 let release!:()=>void,started!:()=>void;const held=new Promise<void>(r=>release=r),requested=new Promise<void>(r=>started=r);let calls=0;
 await page.context().route(`**/canvas/scenes/${a.id}`,async route=>{if(route.request().method()!=='PUT')return route.continue();calls++;if(calls===1){started();await held;}return route.continue();});
 await page.locator('.cm-content').fill('flowchart LR\n A[First queued snapshot] --> B[Save]');await requested;
 await page.locator('.cm-content').fill('flowchart LR\n A[Newest Alpha draft] --> B[Save]');await selectScene(page,b.title);
 await page.getByTitle('Edit the Mermaid source',{exact:true}).click();await page.locator('.cm-content').fill('flowchart LR\n A[Newest Beta draft] --> B[Save]');
 await page.waitForTimeout(650);expect(calls).toBe(1);release();await expect.poll(()=>calls).toBe(2);
 const {ctx,base}=await apiCtx();await expect.poll(async()=>JSON.parse((await (await ctx.get(`${base}/api/v1/canvas/scenes/${a.id}`)).json()).doc_json).source).toContain('Newest Alpha draft');
 await expect.poll(async()=>JSON.parse((await (await ctx.get(`${base}/api/v1/canvas/scenes/${b.id}`)).json()).doc_json).source).toContain('Newest Beta draft');await ctx.dispose();
});
test('Product Discovery ignores late report responses',async({page})=>{
 const {ctx,base}=await apiCtx();const response=await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/product/drafts`,{data:{title:`R3 safe discovery story ${workspaceId}`} });expect(response.ok()).toBeTruthy();const sid=(await response.json()).story.id;await ctx.dispose();
 const run=(id:string)=>({id,story_id:sid,swarm_id:'synthetic',project_id:'synthetic',status:'done',brief_md:'Synthetic review',report_md:`# Report ${id}`,created_by:'fixture',created_at:new Date().toISOString(),updated_at:new Date().toISOString()});
 await page.context().route(`**/product/stories/${sid}/discovery-runs`,route=>route.fulfill({json:['alpha','beta'].map(id=>({run:run(id),derived_status:'done',task_count:2,done_count:2}))}));
 let release!:()=>void,started!:()=>void;const held=new Promise<void>(r=>release=r),requested=new Promise<void>(r=>started=r);
 await page.context().route('**/product/discovery-runs/*',async route=>{const id=route.request().url().split('/').at(-1)!;if(id==='alpha'){started();await held;}return route.fulfill({json:{run:run(id),derived_status:'done',tasks:[],task_summaries:[],messages:[]}});});
 await openPage(page,'product');await page.locator('.story-row',{hasText:`R3 safe discovery story ${workspaceId}`}).click();await page.getByRole('tab',{name:'Discover',exact:true}).click();await page.locator('.sub-tab-strip .st',{hasText:'Discovery'}).click();
 await page.locator('.run-header').nth(0).click();await requested;await page.locator('.run-header').nth(1).click();await expect(page.locator('.run-body')).toContainText('Report beta');const delivered=page.waitForResponse('**/product/discovery-runs/alpha');release();await (await delivered).finished();await page.evaluate(()=>new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve()))));await expect(page.locator('.run-body')).toContainText('Report beta');
 await page.screenshot({path:'/tmp/otto-ux-r3-content-discovery.png'});
});
test('Product dashboard template has readable labels and pin annotations',async({page})=>{
 const {ctx,base}=await apiCtx();const response=await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/product/drafts`,{data:{title:`R3 template story ${workspaceId}`} });expect(response.ok()).toBeTruthy();await ctx.dispose();
 await openPage(page,'product');await page.locator('.story-row',{hasText:`R3 template story ${workspaceId}`}).click();await page.getByRole('tab',{name:'Story',exact:true}).click();await page.locator('.sub-tab-strip .st',{hasText:'Design'}).click();
 const initialPanes=page.getByRole('tablist',{name:'Design panes'});if(await initialPanes.isVisible())await initialPanes.getByRole('tab',{name:'Assets',exact:true}).click();
 await page.getByTitle('New artifact (blank or from a template)').click();await page.getByRole('menuitem',{name:'Template: Dashboard',exact:true}).click();
 const frame=page.frameLocator('iframe.mockup-frame');await expect(frame.locator('.kpi').first()).toBeVisible();
 expect((await page.locator('.arena-stage').boundingBox())!.width).toBeGreaterThan(400);
 expect(await frame.locator('.kpi small').first().evaluate(el=>parseFloat(getComputedStyle(el).fontSize))).toBeGreaterThanOrEqual(11);
 await page.screenshot({path:'/tmp/otto-ux-r3-content-product-dashboard.png'});
 const panes=page.getByRole('tablist',{name:'Design panes'});await panes.getByRole('tab',{name:'Canvas',exact:true}).focus();await page.keyboard.press('End');await expect(panes.getByRole('tab',{name:'Inspector',exact:true})).toBeFocused();await page.keyboard.press('Home');await expect(panes.getByRole('tab',{name:'Assets',exact:true})).toBeFocused();await panes.getByRole('tab',{name:'Canvas',exact:true}).click();
 await page.setViewportSize({width:1800,height:900});await expect(frame.locator('.kpi').first()).toBeVisible();await page.setViewportSize({width:1024,height:900});await expect(frame.locator('.kpi').first()).toBeVisible();
 const overlay=page.locator('.overlay.annotate');await overlay.click({position:{x:120,y:90}});await page.locator('.editor textarea').fill('Please align this card');await page.locator('.editor').getByRole('button',{name:'Add',exact:true}).click();await expect(page.locator('.editor')).toHaveCount(0);
});
test('Canvas Excalidraw preserves failed hand edits across scene switching',async({page})=>{
 const {ctx,base}=await apiCtx();const response=await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/canvas/scenes`,{data:{title:`R3 editable Excalidraw ${workspaceId}`,doc:{type:'otto-canvas',version:1,format:'excalidraw',source:JSON.stringify({type:'excalidraw',elements:[]})}}});expect(response.ok()).toBeTruthy();const id=(await response.json()).id;await ctx.dispose();const [,b]=await scenes();
 await openPage(page,'canvas');await page.locator('.scene-list .row',{hasText:`R3 editable Excalidraw ${workspaceId}`}).getByRole('button').first().click();const drawing=page.locator('.excalidraw canvas').first();await expect(drawing).toBeVisible({timeout:30000});
 let writes=0;await page.context().route(`**/canvas/scenes/${id}`,route=>{if(route.request().method()!=='PUT')return route.continue();writes++;return route.fulfill({status:503,json:{code:'upstream',message:'Drawing save unavailable'}});});
 const box=(await drawing.boundingBox())!;await page.mouse.click(box.x+box.width*.5,box.y+box.height*.5);await page.keyboard.press('r');await page.mouse.move(box.x+box.width*.4,box.y+box.height*.4);await page.mouse.down();await page.mouse.move(box.x+box.width*.6,box.y+box.height*.6,{steps:8});await page.mouse.up();
 await selectScene(page,b.title);await expect.poll(()=>writes).toBeGreaterThan(0);await page.locator('.scene-list .row',{hasText:`R3 editable Excalidraw ${workspaceId}`}).getByRole('button').first().click();await expect(drawing).toBeVisible();
 await expect(page.locator('.save-error')).toContainText('Drawing save unavailable');
 await expect.poll(()=>page.evaluate(async()=>{const path='/src/lib/stores/canvas.svelte.ts';return (await import(path)).canvas.source;})).toContain('rectangle');await page.screenshot({path:'/tmp/otto-ux-r3-content-excalidraw.png'});
});
test('Snip transient image failure offers Retry instead of claiming deletion',async({page})=>{
 let failing=true;await page.context().route('**/snips/synthetic-retry/image',route=>failing?route.fulfill({status:503,json:{code:'upstream',message:'Image temporarily unavailable'}}):route.fulfill({status:404,json:{code:'not_found',message:'Not found'}}));
 await page.goto('/#/snip/synthetic-retry');await expect(page.getByRole('alert')).toContainText('Image temporarily unavailable');
 await expect(page.getByText('This snip no longer exists')).toHaveCount(0);failing=false;await page.getByRole('button',{name:'Retry',exact:true}).click();await expect(page.locator('.snip-missing')).toContainText('This snip no longer exists');
});
test('Snip pending copy remains attached to its original image when switching snips',async({page})=>{
 const png=await page.evaluate(()=>{const c=document.createElement('canvas');c.width=300;c.height=200;return c.toDataURL().split(',')[1];});const {ctx,base}=await apiCtx();const ids:string[]=[];for(let i=0;i<2;i++){const r=await ctx.post(`${base}/api/v1/snips`,{data:{data_b64:png,filename:`switch-${i}.png`}});expect(r.ok()).toBeTruthy();ids.push((await r.json()).id);}await ctx.dispose();
 const copies:string[]=[];let copiedPng='';await page.context().route('**/snips/*/annotated',route=>{copiedPng=route.request().postDataJSON().data_b64;copies.push(route.request().url().split('/').at(-2)!);return route.fulfill({json:{copied:true}});});
 await page.goto(`/#/snip/${ids[0]}`);const c=page.locator('.snip-canvas');await expect(c).toBeVisible();const b=(await c.boundingBox())!;await page.mouse.move(b.x+25,b.y+25);await page.mouse.down();await page.mouse.move(b.x+110,b.y+110,{steps:3});await page.mouse.up();await page.evaluate(id=>{location.hash=`/snip/${id}`;},ids[1]);await expect(page.locator('.snip-editor')).toHaveAttribute('data-count','0');await expect.poll(()=>copies).toEqual([ids[0]]);
 const pixels=await page.evaluate(data=>new Promise<{width:number;height:number;painted:boolean}>(resolve=>{const image=new Image();image.onload=()=>{const canvas=document.createElement('canvas');canvas.width=image.width;canvas.height=image.height;const context=canvas.getContext('2d')!;context.drawImage(image,0,0);const rgba=context.getImageData(0,0,canvas.width,canvas.height).data;resolve({width:canvas.width,height:canvas.height,painted:rgba.some((value,index)=>index%4===3&&value>0)});};image.src=`data:image/png;base64,${data}`;}),copiedPng);
 expect(pixels).toEqual({width:300,height:200,painted:true});
});
