import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage, openApiEditor, expectNoHorizontalOverflow } from './helpers';
import { mkdirSync } from 'node:fs';
test.use({serviceWorkers:'block'});
test.setTimeout(45000);
async function setup(page: Page, kind: 'broker' | 'database') {
  const {ctx, base} = await apiCtx();
  const id = await seedWorkspace(ctx, base);
  await page.addInitScript(id => { localStorage.setItem('otto_workspace', id); localStorage.setItem('otto_firstrun_dismissed', '1'); }, id);
  const response = await ctx.post(`${base}/api/v1/workspaces/${id}/${kind === 'broker' ? 'brokers/clusters' : 'connections'}`, {data: kind === 'broker' ? {name:`Review Kafka ${id}`,bootstrap_servers:'fixture.invalid:9092',security_protocol:'plaintext',environment:'dev',read_only:false} : {name:`Review SQL ${id}`,kind:'mysql',params:{host:'fixture.invalid',port:3306}}});
  expect(response.ok()).toBeTruthy();
  const entity = await response.json();
  await ctx.dispose();
  if (kind === 'broker') {
    await page.route('**/brokers/clusters/*/overview', r => r.fulfill({json:{brokers:[],topic_count:2,internal_topic_count:0,partition_count:2,consumer_group_count:2,controller_id:0}}));
    await page.route('**/brokers/clusters/*/test', r => r.fulfill({json:{ok:true,latency_ms:1}}));
    await page.route('**/brokers/clusters/*/metrics', r => r.fulfill({json:{messages_per_sec:0,throughput:[]}}));
    await openPage(page, 'brokers');
    await page.locator('.cluster .cn', {hasText:`Review Kafka ${id}`} ).click();
  } else {
    await page.route('**/db/capabilities', r => r.fulfill({json:{engine:'mysql',query_language:'sql',version:'8.0',can_write:true,explain:true}}));
    await page.route('**/db/schema**', r => r.fulfill({json:[]}));
    await openPage(page,'database');
    await page.locator('.conn-list .conn-name', {hasText:`Review SQL ${id}`} ).first().click();
    await expect(page.getByRole('tablist', {name:'Query tabs'})).toBeVisible();
  }
  return entity;
}


test('Database RTL arrows follow the visual order of sidebar, query and main tabs', async ({page}) => {
  await page.addInitScript(() => localStorage.setItem('otto_direction','rtl'));
  await setup(page,'database');
  await expect(page.locator('.qe-edit .cm-content')).toHaveCSS('direction','ltr');
  const sidebar = page.getByRole('tablist',{name:'Sidebar view'});
  await sidebar.getByRole('tab',{name:'Schema',exact:true}).click();
  await sidebar.getByRole('tab',{name:'Schema',exact:true}).press('ArrowLeft');
  await expect(sidebar.getByRole('tab',{name:'Saved',exact:true})).toBeFocused();
  await page.getByRole('button',{name:'New query tab',exact:true}).click();
  await page.getByRole('button',{name:'New query tab',exact:true}).click();
  const queries = page.getByRole('tablist',{name:'Query tabs'}).getByRole('tab');
  await queries.first().click(); await queries.first().press('ArrowLeft');
  await expect(queries.nth(1)).toBeFocused();
  const main = page.locator('.main-tabs');
  await main.getByRole('tab',{name:'Query',exact:true}).click();
  await main.getByRole('tab',{name:'Query',exact:true}).press('ArrowLeft');
  await expect(main.getByRole('tab',{name:'Structure',exact:true})).toBeFocused();
});

test('Phone schema selection exposes one keyboard reachable selected tab', async ({page}) => {
  await setup(page,'database');
  await page.getByRole('tablist',{name:'Sidebar view'}).getByRole('tab',{name:'Connections',exact:true}).click();
  await page.setViewportSize({width:375,height:812});
  const toggle = page.getByRole('button',{name:'Schema & saved',exact:true});
  if (await toggle.getAttribute('aria-expanded') === 'false') await toggle.click();
  const sidebar = page.getByRole('tablist',{name:'Sidebar view'});
  await expect(sidebar.getByRole('tab',{name:'Schema',exact:true})).toHaveClass(/active/);
  await expect(sidebar.getByRole('tab',{name:'Schema',exact:true})).toHaveAttribute('aria-selected','true');
  await expect(sidebar.locator('[tabindex="0"]')).toHaveCount(1);
});

test('Saving a query preserves its original tab when another query becomes active', async ({page}) => {
  let release!: () => void;
  const held = new Promise<void>(resolve => release = resolve);
  let sent = false;
  await page.route('**/saved-queries', async r => {
    if (r.request().method() !== 'POST') return r.fulfill({json:[]});
    sent = true; await held;
    return r.fulfill({json:{...r.request().postDataJSON(),id:'saved-original',workspace_id:'fixture',created_at:'2026-09-25'}});
  });
  await setup(page,'database');
  await page.locator('.qe-edit .cm-content').fill('SELECT 41');
  await page.getByRole('button',{name:'Save',exact:true}).click();
  await page.getByPlaceholder('Query name').fill('Original query');
  await page.locator('.save-bar').getByRole('button',{name:'Save',exact:true}).click();
  await expect.poll(() => sent).toBe(true);
  await page.getByRole('button',{name:'New query tab',exact:true}).click();
  await page.locator('.qe-edit .cm-content').fill('SELECT 42');
  const done = page.waitForResponse(r => r.url().endsWith('/saved-queries') && r.request().method()==='POST');
  release(); await done;
  const tabs = page.getByRole('tablist',{name:'Query tabs'}).getByRole('tab');
  await expect(tabs.first()).toContainText('Original query');
  await expect(tabs.last()).not.toContainText('Original query');
  await expect(page.locator('.qe-edit .cm-content')).toHaveText('SELECT 42');
});

test('Consumer groups adapt to their available width and resize using RTL keyboard arrows', async ({page}) => {
  await page.setViewportSize({width:1440,height:900});
  await page.addInitScript(() => localStorage.setItem('otto_direction','rtl'));
  await page.route('**/groups', r => r.fulfill({json:[{group_id:'customer-notification-consumers',state:'Stable',members:2}]}));
  await page.route('**/groups/*', r => r.fulfill({json:{group_id:'customer-notification-consumers',state:'Stable',members:[],offsets:[{topic:'customer-notifications',partition:0,current_offset:98432,high_watermark:98652,lag:220}],total_lag:220}}));
  await setup(page,'broker');
  await page.locator('.tabs button',{hasText:'Groups'}).click();
  await expect(page.locator('.detail .gid.big')).toBeVisible();
  const groups = page.locator('.groups');
  const separator = groups.getByRole('separator');
  await expect.soft(separator).toHaveAttribute('tabindex','0');
  const before = await groups.locator('.list').evaluate(e=>e.getBoundingClientRect().width);
  if (await separator.getAttribute('tabindex') === '0') {
    await separator.focus(); await separator.press('ArrowLeft');
    await expect.poll(()=>groups.locator('.list').evaluate(e=>e.getBoundingClientRect().width)).toBeGreaterThan(before);
  }
  // Same desktop viewport, narrowed content as in an app split pane.
  await groups.evaluate(e => {e.parentElement!.style.width='620px';});
  const listBox = await groups.locator('.list').boundingBox();
  const detailBox = await groups.locator('.detail').boundingBox();
  expect(detailBox!.y).toBeGreaterThanOrEqual(listBox!.y + listBox!.height - 1);
  await expect(separator).toBeHidden();
  await expectNoHorizontalOverflow(page);
  mkdirSync('/tmp/otto-ux-r3-data-screenshots',{recursive:true});
  await page.screenshot({animations:'disabled',path:'/tmp/otto-ux-r3-data-screenshots/groups-narrow-rtl.png'});
});

test('Network profile refuses invalid TCP ports before submission', async ({page}) => {
  await page.addInitScript(() => {localStorage.setItem('otto_base',location.origin);localStorage.setItem('otto_token','fixture');});
  await page.route('**/api/v1/**', r => r.fulfill({json:r.request().url().endsWith('/connections')?[{id:'ssh',name:'Fixture bastion',kind:'ssh'}]:[]}));
  await page.goto('/e2e/fixtures/network-profiles.html');
  await page.getByRole('button',{name:'Manage network profiles'}).click();
  await page.getByLabel('Network profile name').fill('Fixture profile');
  await page.getByLabel('SSH connection').selectOption('ssh');
  await page.getByLabel('Endpoint 1 name').fill('Database');
  await page.getByLabel('Endpoint 1 remote host').fill('db.fixture.invalid');
  for (const value of ['-1','65536','1.5','0','']) {
    await page.getByLabel('Endpoint 1 remote port').fill(value);
    await expect(page.getByRole('button',{name:'Save network profile',exact:true})).toBeDisabled();
  }
  await page.getByLabel('Endpoint 1 remote port').fill('5432');
  await expect(page.getByRole('button',{name:'Save network profile',exact:true})).toBeEnabled();
});

test('Completed offset reset cannot replace another selected group', async ({page}) => {
  let release!: () => void;
  const held = new Promise<void>(resolve => release=resolve);
  const detail = (id:string) => ({group_id:id,state:'Stable',members:[],offsets:[],total_lag:0});
  await page.route('**/groups',r=>r.fulfill({json:[{group_id:'alpha',state:'Stable',members:0},{group_id:'beta',state:'Stable',members:0}]}));
  await page.route('**/groups/*',r=>r.fulfill({json:detail(r.request().url().split('/').pop()!)}));
  let started=false;
  await page.route('**/groups/alpha/reset',async r=>{started=true;await held;return r.fulfill({json:detail('alpha')});});
  await setup(page,'broker');
  await page.locator('.tabs button',{hasText:'Groups'}).click();
  await expect(page.locator('.detail .gid.big')).toHaveText('alpha');
  await page.getByRole('button',{name:'Reset',exact:true}).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('textbox').fill('alpha');
  await dialog.getByRole('button',{name:'Reset',exact:true}).click();
  await expect.poll(()=>started).toBe(true);
  await page.locator('.grow-row',{hasText:'beta'}).click();
  await expect(page.locator('.detail .gid.big')).toHaveText('beta');
  const done=page.waitForResponse(r=>r.url().endsWith('/groups/alpha/reset'));
  release(); await done;
  await expect(page.locator('.detail .gid.big')).toHaveText('beta');
});

for (const variant of [
  {name:'native-light',theme:'native',scheme:'light',direction:'ltr',width:1440,height:900},
  {name:'native-dark',theme:'native',scheme:'dark',direction:'ltr',width:1440,height:900},
  {name:'warm-light',theme:'warm',scheme:'light',direction:'ltr',width:1440,height:900},
  {name:'warm-dark',theme:'warm',scheme:'dark',direction:'ltr',width:1440,height:900},
  {name:'pro-dark',theme:'pro-dark',scheme:'dark',direction:'ltr',width:1440,height:900},
  {name:'phone',theme:'native',scheme:'light',direction:'ltr',width:375,height:812},
  {name:'tablet-rtl',theme:'warm',scheme:'dark',direction:'rtl',width:834,height:1112},
]) test(`Network profiles loaded ${variant.name}`, async ({page}) => {
  await page.setViewportSize(variant);
  await page.addInitScript(v=>{localStorage.setItem('otto_base',location.origin);localStorage.setItem('otto_token','fixture');localStorage.setItem('otto_theme',v.theme);localStorage.setItem('otto_scheme',v.scheme);localStorage.setItem('otto_direction',v.direction);},variant);
  const profile = {id:'fixture',name:'Customer reporting database',workspace_id:'w',version:1,archived:false,ssh_connection_id:'ssh',endpoints:[{name:'Reporting database',remote_host:'reporting-database.fixture.internal',remote_port:5432,host_env:'PGHOST',port_env:'PGPORT'}]};
  await page.route('**/api/v1/**',r=>r.fulfill({json:r.request().url().endsWith('/connections')?[{id:'ssh',name:'Private reporting bastion',kind:'ssh'}]:r.request().url().endsWith('/network-profiles')?[profile]:[]}));
  await page.goto('/e2e/fixtures/network-profiles.html');
  await page.getByRole('button',{name:'Manage network profiles'}).click();
  await page.getByRole('button',{name:'Edit Customer reporting database'}).click();
  await expect(page.getByLabel('Endpoint 1 remote host')).toHaveValue('reporting-database.fixture.internal');
  for (const field of ['Endpoint 1 remote host','Endpoint 1 host environment','Endpoint 1 port environment']) {
    await expect(page.getByLabel(field)).toHaveCSS('direction','ltr');
  }
  await page.getByRole('button',{name:'Save network profile',exact:true}).scrollIntoViewIfNeeded();
  await expect(page.getByRole('button',{name:'Save network profile',exact:true})).toBeEnabled();
  await expectNoHorizontalOverflow(page);
  await page.locator('.manager').evaluate(e=>{e.scrollTop=0;});
  mkdirSync('/tmp/otto-ux-r3-data-screenshots',{recursive:true});
  await page.screenshot({animations:'disabled',path:`/tmp/otto-ux-r3-data-screenshots/network-${variant.name}.png`});
});

test('Saved query retry cannot replace the next workspace list', async ({page}) => {
  const {ctx,base}=await apiCtx(); await seedWorkspace(ctx,base); await ctx.dispose();
  let original=''; let phase='fail'; let release!:()=>void;
  const held=new Promise<void>(resolve=>release=resolve);
  await page.route('**/saved-queries',async r=>{
    const url=r.request().url(); if (!original) original=url;
    if (url===original && phase==='fail') return r.fulfill({status:503,json:{code:'upstream',message:'Fixture saved list unavailable'}});
    if (url===original) await held;
    return r.fulfill({json:[{id:url===original?'old':'new',name:url===original?'Previous workspace query':'Current workspace query',statement:'SELECT 1',workspace_id:url.split('/')[6],connection_id:null}]});
  });
  await setup(page,'database');
  await page.getByRole('tablist',{name:'Sidebar view'}).getByRole('tab',{name:'Saved',exact:true}).click();
  phase='hold';
  await page.locator('.side-body').getByRole('button',{name:'Retry',exact:true}).click();
  await page.getByRole('button',{name:'E2E WS',exact:true}).first().click();
  await page.getByRole('tablist',{name:'Sidebar view'}).getByRole('tab',{name:'Saved',exact:true}).click();
  await expect(page.getByRole('button',{name:'Current workspace query',exact:true})).toBeVisible();
  const done=page.waitForResponse(r=>r.url()===original);
  release(); await done;
  await expect(page.getByRole('button',{name:'Current workspace query',exact:true})).toBeVisible();
  await expect(page.getByRole('button',{name:'Previous workspace query',exact:true})).toHaveCount(0);
});

test('History retry ignores the connection left behind', async ({page}) => {
  const {ctx,base}=await apiCtx(); const wid=await seedWorkspace(ctx,base);
  const alternate=`History alternate ${wid}`;
  expect((await ctx.post(`${base}/api/v1/workspaces/${wid}/connections`,{data:{name:alternate,kind:'mysql',params:{host:'fixture.invalid',port:3306}}})).ok()).toBeTruthy();
  await ctx.dispose();
  let first=''; let phase='fail'; let release!:()=>void;
  const held=new Promise<void>(resolve=>release=resolve);
  await page.route('**/db/history**',async r=>{
    const url=r.request().url(); if(!first) first=url;
    if (url===first && phase==='fail') return r.fulfill({status:503,json:{code:'upstream',message:'Fixture history unavailable'}});
    if(url===first) await held;
    return r.fulfill({json:[{id:url===first?'old':'new',connection_id:'fixture',statement:url===first?'SELECT previous_connection':'SELECT current_connection',ok:true,duration_ms:1,row_count:1,created_at:'2026-09-25T00:00:00Z'}]});
  });
  await setup(page,'database');
  const sidebar=page.getByRole('tablist',{name:'Sidebar view'});
  await sidebar.getByRole('tab',{name:'History',exact:true}).click();
  phase='hold'; await page.locator('.side-body').getByRole('button',{name:'Retry',exact:true}).click();
  await sidebar.getByRole('tab',{name:'Connections',exact:true}).click();
  // Every profile was seeded into this disposable daemon by this suite.
  await page.locator('.conn-list .conn-name').filter({hasText:alternate}).click();
  await sidebar.getByRole('tab',{name:'History',exact:true}).click();
  await expect(page.locator('.hist-row')).toContainText('SELECT current_connection');
  const done=page.waitForResponse(r=>r.url()===first); release(); await done;
  await expect(page.locator('.hist-row')).toContainText('SELECT current_connection');
});

test('Large schema search opens the matching structure without page overflow', async ({page}) => {
  await page.setViewportSize({width:834,height:1112});
  await setup(page,'database');
  await page.locator('.qe-edit .cm-content').fill('SELECT preserved_tablet_draft');
  const objects=Array.from({length:120},(_,i)=>({id:`reporting.customer_reporting_${i}`,label:`customer_reporting_${i}`,kind:'table',has_children:false}));
  await page.route('**/db/schema**',r=>r.fulfill({json:objects}));
  await page.route('**/db/search-objects',r=>r.fulfill({json:{hits:[{schema:'reporting',name:'customer_reporting_119',path:'reporting.customer_reporting_119',kind:'table'}],truncated:false,scanned:1,supported:true}}));
  await page.route('**/db/object',r=>r.fulfill({json:{name:'customer_reporting_119',kind:'table',columns:[{name:'customer_reference_identifier',data_type:'VARCHAR(255)',nullable:false}],indexes:[],foreign_keys:[],primary_key:[]}}));
  await page.getByRole('button',{name:'Refresh schema',exact:true}).click();
  await expect(page.getByRole('tree',{name:'Schema'}).getByRole('treeitem')).toHaveCount(120);
  await page.getByRole('textbox',{name:'Find an object'}).fill('customer_reporting_119');
  await page.locator('.hit').filter({hasText:'customer_reporting_119'}).click();
  await expect(page.getByRole('tablist',{name:'Workbench view'}).getByRole('tab',{name:'Structure',exact:true})).toHaveAttribute('aria-selected','true');
  await expect(page.getByText('customer_reference_identifier',{exact:true})).toBeVisible();
  await expect(page.getByRole('tablist',{name:'Workbench view'}).getByRole('tab',{name:'Structure',exact:true})).toBeFocused();
  expect((await page.locator('.structure').boundingBox())!.width).toBeGreaterThan(500);
  await expectNoHorizontalOverflow(page);
  await page.screenshot({animations:'disabled',path:'/tmp/otto-ux-r3-data-screenshots/schema-tablet.png'});
  await page.getByRole('button',{name:'Show schema sidebar',exact:true}).click();
  await expect(page.getByRole('textbox',{name:'Find an object'})).toHaveValue('customer_reporting_119');
  await expect(page.locator('.hit').filter({hasText:'customer_reporting_119'})).toBeFocused();
  await page.getByRole('tablist',{name:'Workbench view'}).getByRole('tab',{name:'Query',exact:true}).click();
  for (const size of [{width:1440,height:900},{width:375,height:812}]) {
    await page.setViewportSize(size);
    await expect(page.locator('.qe-edit .cm-content')).toHaveText('SELECT preserved_tablet_draft');
    await expectNoHorizontalOverflow(page);
  }
});


test('RTL API JSON response keeps code direction while chrome mirrors', async ({page}) => {
  await page.addInitScript(()=>localStorage.setItem('otto_direction','rtl'));
  await setup(page,'database');
  await page.route('**/api-client/execute',r=>r.fulfill({json:{status:200,status_text:'OK',headers:[{key:'Content-Type',value:'application/json'}],body:JSON.stringify({customer:'Fixture',count:2}),body_base64:'',truncated:false,too_large:false,duration_ms:2,size_bytes:32,content_type:'application/json',trace:[]}}));
  await openApiEditor(page);
  await page.getByLabel('Request URL',{exact:true}).fill('https://fixture.invalid/customers');
  await page.getByRole('button',{name:'Send',exact:true}).click();
  await page.getByRole('button',{name:'Pretty',exact:true}).click();
  await expect(page.locator('.cm-content')).toHaveCSS('direction','ltr');
  await expect(page.locator('html')).toHaveAttribute('dir','rtl');
  await expect(page.locator('.cm-content')).toContainText('customer');
});
