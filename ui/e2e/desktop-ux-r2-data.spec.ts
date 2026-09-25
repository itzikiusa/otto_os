import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage, openApiEditor, expectNoHorizontalOverflow, expectFullyInViewport } from './helpers';
import { mkdirSync } from 'node:fs';
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

test('Replay rejects blank timestamps and out of range numeric selectors', async ({page}) => {
  await setup(page,'broker');
  await page.locator('.tabs button', {hasText:'Replay'}).click();
  await page.getByLabel('Source topic').fill('orders-dlq');
  await page.getByLabel('Target topic').fill('orders');
  const replay = page.getByRole('button', {name:'Replay',exact:true});
  for (const value of ['', '0', '1.5', '5001']) { await page.getByLabel('Count', {exact:true}).fill(value); await expect(replay).toBeDisabled(); }
  await page.getByRole('combobox', {name:'Selector',exact:true}).selectOption('timestamp');
  await expect(replay).toBeDisabled();
  await expect(page.locator('.field-err')).toContainText('time');
  await page.getByLabel('From (local time)').fill('2026-09-25T10:00');
  await expect(replay).toBeEnabled();
  await page.getByLabel('Limit', {exact:true}).fill('-1');
  await expect(replay).toBeDisabled();
  await page.getByRole('combobox', {name:'Selector',exact:true}).selectOption('offset_range');
  await page.getByLabel('Partition', {exact:true}).fill('-1');
  await expect(replay).toBeDisabled();
});

test('Groups ignores delayed details after choosing another group', async ({page}) => {
  let release!: () => void;
  const held = new Promise<void>(resolve => release = resolve);
  await page.route('**/groups', r => r.fulfill({json:[{group_id:'alpha',state:'Stable',members:1},{group_id:'beta',state:'Stable',members:2}]}));
  await page.route('**/groups/*', async r => {
    const group_id = r.request().url().split('/').pop();
    if (group_id === 'alpha') await held;
    return r.fulfill({json:{group_id,state:'Stable',members:[],offsets:[],total_lag:0}});
  });
  await setup(page,'broker');
  await page.locator('.tabs button', {hasText:'Groups'}).click();
  await expect(page.getByText('Loading group…', {exact:true})).toBeVisible();
  await page.locator('.grow-row', {hasText:'beta'}).click();
  await expect(page.locator('.detail .gid.big')).toHaveText('beta');
  const done = page.waitForResponse(r => r.url().endsWith('/groups/alpha'));
  release(); await done;
  await expect(page.locator('.detail .gid.big')).toHaveText('beta');
  await page.getByLabel('Reset to').selectOption('timestamp');
  await expect(page.getByRole('button', {name:'Preview',exact:true})).toBeDisabled();
});

test('Groups discards an old reset preview after switching groups', async ({page}) => {
  let release!: () => void;
  const held = new Promise<void>(resolve => release = resolve);
  await page.route('**/groups', r => r.fulfill({json:[{group_id:'alpha',state:'Stable',members:1},{group_id:'beta',state:'Stable',members:2}]}));
  await page.route('**/groups/*', r => r.fulfill({json:{group_id:r.request().url().split('/').pop(),state:'Stable',members:[],offsets:[],total_lag:0}}));
  await page.route('**/groups/alpha/reset?dry_run=true', async r => {await held; return r.fulfill({json:{total_lag_before:100,total_lag_after:0,partitions:[]}});});
  await setup(page,'broker');
  await page.locator('.tabs button',{hasText:'Groups'}).click();
  await expect(page.locator('.detail .gid.big')).toHaveText('alpha');
  await page.getByRole('button',{name:'Preview',exact:true}).click();
  await page.locator('.grow-row',{hasText:'beta'}).click();
  await expect(page.locator('.detail .gid.big')).toHaveText('beta');
  const done = page.waitForResponse(r => r.url().endsWith('/groups/alpha/reset?dry_run=true'));
  release(); await done;
  await expect(page.locator('.dryrun-preview')).toHaveCount(0);
});

test('Network profile distinguishes loading, failure and empty SSH lists', async ({page}) => {
  let release!: () => void;
  const held = new Promise<void>(resolve => release = resolve);
  let fail = true;
  await page.addInitScript(() => {localStorage.setItem('otto_base',location.origin);localStorage.setItem('otto_token','fixture');});
  await page.route('**/api/v1/**', async r => {
    if (r.request().url().endsWith('/connections')) { await held; return fail ? r.fulfill({status:503,json:{code:'upstream',message:'SSH profiles unavailable'}}) : r.fulfill({json:[]}); }
    return r.fulfill({json:[]});
  });
  await page.goto('/e2e/fixtures/network-profiles.html');
  await page.getByRole('button', {name:'Manage network profiles'}).click();
  await expect(page.getByText('Create an SSH connection in Connections before saving a network profile.')).toHaveCount(0);
  await expect(page.getByLabel('Loading SSH connections', {exact:true})).toBeVisible();
  release();
  await expect(page.getByText('SSH profiles unavailable', {exact:true})).toBeVisible();
  await expect(page.getByText('Create an SSH connection in Connections before saving a network profile.')).toHaveCount(0);
  fail = false;
  await page.getByRole('button', {name:'Retry',exact:true}).click();
  await expect(page.getByText('Create an SSH connection in Connections before saving a network profile.')).toBeVisible();
});

test('Database saved and history failures offer retry; tabs follow keyboard focus', async ({page}) => {
  let fail = true;
  for (const url of ['**/saved-queries','**/db/history**']) await page.route(url, r => fail ? r.fulfill({status:503,json:{code:'upstream',message:'Fixture list unavailable'}}) : r.fulfill({json:[]}));
  await setup(page,'database');
  const tabs = page.getByRole('tablist',{name:'Sidebar view'});
  await tabs.getByRole('tab',{name:'Saved',exact:true}).click();
  await expect(page.locator('.side-body').getByRole('button',{name:'Retry',exact:true})).toBeVisible();
  await expect(page.getByText('No saved queries. Save one from the Query tab.')).toHaveCount(0);
  fail = false;
  await page.locator('.side-body').getByRole('button',{name:'Retry',exact:true}).click();
  await expect(page.getByText('No saved queries. Save one from the Query tab.')).toBeVisible();
  await tabs.getByRole('tab',{name:'History',exact:true}).click();
  await expect(page.locator('.side-body').getByRole('button',{name:'Retry',exact:true})).toBeVisible();
  await page.locator('.side-body').getByRole('button',{name:'Retry',exact:true}).click();
  await expect(page.getByText('No query history yet.')).toBeVisible();
  await tabs.getByRole('tab',{name:'History',exact:true}).press('Home');
  await expect(tabs.getByRole('tab',{name:'Connections',exact:true})).toBeFocused();
  await page.getByRole('button', {name:'New query tab',exact:true}).click();
  const queries = page.getByRole('tablist',{name:'Query tabs'}).getByRole('tab');
  await queries.last().focus(); await queries.last().press('Home');
  await expect(queries.first()).toBeFocused();
  await expect(queries.first()).toHaveAttribute('aria-selected','true');
  await queries.first().press('End'); await expect(queries.last()).toBeFocused();
});

const variants = [
  {name:'native-light',theme:'native',scheme:'light',direction:'ltr',width:1440,height:900},
  {name:'native-dark',theme:'native',scheme:'dark',direction:'ltr',width:1440,height:900},
  {name:'warm-light',theme:'warm',scheme:'light',direction:'ltr',width:1440,height:900},
  {name:'warm-dark',theme:'warm',scheme:'dark',direction:'ltr',width:1440,height:900},
  {name:'pro-dark',theme:'pro-dark',scheme:'dark',direction:'ltr',width:1440,height:900},
  {name:'phone',theme:'native',scheme:'light',direction:'ltr',width:375,height:812},
  {name:'tablet-rtl',theme:'warm',scheme:'dark',direction:'rtl',width:1024,height:768},
];
for (const variant of variants) test(`Loaded data workbenches ${variant.name}`, async ({page}) => {
  test.setTimeout(90000);
  await page.setViewportSize(variant);
  await page.addInitScript(v => {localStorage.setItem('otto_theme',v.theme);localStorage.setItem('otto_scheme',v.scheme);localStorage.setItem('otto_direction',v.direction);}, variant);
  mkdirSync('/tmp/otto-ux-r2-data-screenshots',{recursive:true});
  async function shot(name:string) {
    await expectNoHorizontalOverflow(page);
    await page.screenshot({animations:'disabled',path:`/tmp/otto-ux-r2-data-screenshots/${variant.name}-${name}.png`});
  }
  await page.route('**/db/query', r => r.fulfill({json:{columns:[{name:'id',type_hint:'INT'},{name:'customer',type_hint:'VARCHAR'},{name:'notes',type_hint:'TEXT'}],rows:[[1,'Example customer','A detailed customer note with readable text'],[2,'Long customer name for layout validation','Second note']],stats:{duration_ms:12,row_count:2},truncated:false}}));
  await page.route('**/db/history**', r => r.fulfill({json:[]}));
  await setup(page,'database');
  const editor = page.locator('.qe-edit .cm-content');
  await editor.fill('SELECT id, customer, notes FROM customers LIMIT 20');
  await page.getByRole('button',{name:/^Run\s+⌘/}).click();
  await expect(page.getByText('Example customer',{exact:true})).toBeVisible();
  if (variant.name === 'phone') await page.getByRole('tab',{name:'Vertical',exact:true}).click();
  await page.getByText('Example customer',{exact:true}).scrollIntoViewIfNeeded();
  await shot('database');

  await page.route('**/api-client/execute', r => r.fulfill({json:{status:200,status_text:'OK',headers:[{key:'Content-Type',value:'application/json'}],body:JSON.stringify({customers:[{id:1,name:'Example customer',notes:'Multiline response\nSecond line'}],total:1}),body_base64:'',truncated:false,too_large:false,duration_ms:19,size_bytes:122,content_type:'application/json',trace:[{label:'Response',detail:'Fixture JSON response',ms:19,level:'success'}]}}));
  await openApiEditor(page);
  await page.getByLabel('Request URL',{exact:true}).fill('https://fixture.invalid/customers');
  await page.getByRole('button',{name:'Send',exact:true}).click();
  await expect(page.getByRole('button',{name:'Tree',exact:true})).toBeVisible();
  await page.getByRole('button',{name:'Tree',exact:true}).click();
  await expect(page.getByText('Loading saved requests…',{exact:true})).toHaveCount(0);
  await shot('api');

  await page.route('**/groups', r => r.fulfill({json:[{group_id:'customer-notification-consumers',state:'Stable',members:2},{group_id:'audit-log-consumers',state:'Empty',members:0}]}));
  await page.route('**/groups/*', r => r.fulfill({json:{group_id:r.request().url().split('/').pop(),state:'Stable',members:[],offsets:[{topic:'customer-notifications',partition:0,current_offset:98432,high_watermark:98652,lag:220}],total_lag:220}}));
  await setup(page,'broker');
  await page.locator('.tabs button',{hasText:'Groups'}).click();
  await expect(page.locator('.detail .gid.big')).toHaveText('customer-notification-consumers');
  await shot('brokers');
  await page.locator('.tabs button',{hasText:'Replay'}).click();
  await page.getByLabel('Source topic').fill('customer-notifications-dlq');
  await page.getByLabel('Target topic').fill('customer-notifications');
  await page.getByRole('combobox',{name:'Selector',exact:true}).selectOption('timestamp');
  await expect(page.getByRole('button',{name:'Replay',exact:true})).toBeDisabled();
  await shot('replay');

  const {ctx,base} = await apiCtx();
  const id = await seedWorkspace(ctx,base);
  const response = await ctx.post(`${base}/api/v1/workspaces/${id}/connections`,{data:{name:`SFTP ${id}`,kind:'ssh',params:{host:'fixture.invalid'}}});
  expect(response.ok()).toBeTruthy();
  const ssh = await response.json(); await ctx.dispose();
  await page.route(`**/connections/${ssh.id}/sftp/list**`, r => r.fulfill({json:{path:'/home/reports',entries:[{name:'customer-account-exports-with-an-extremely-long-descriptive-filename-2026.csv',kind:'file',size:1048576,mtime:'2026-09-25',perms:'-rw-r--r--',symlink_target:null},{name:'monthly-reports',kind:'dir',size:0,mtime:'2026-09-25',perms:'drwxr-xr-x',symlink_target:null}]}}));
  await page.route(`**/connections/${ssh.id}/sftp/transfers`,r => r.fulfill({json:[]}));
  await openPage(page,'connections');
  const picker = page.getByRole('tab', {name:'Connections',exact:true});
  if (await picker.isVisible()) await picker.click();
  const accordion = page.getByRole('button', {name:/^Connections \d+/});
  if (await accordion.isVisible() && await accordion.getAttribute('aria-expanded') === 'false') await accordion.click();
  await page.locator('.conn-row',{hasText:`SFTP ${id}`}).click({button:'right'});
  await page.getByRole('menuitem',{name:'Browse files (SFTP)',exact:true}).click();
  await expect(page.locator('.sftp .nav',{hasText:'monthly-reports'})).toBeVisible();
  await expectFullyInViewport(page,page.getByLabel('Filter files in this directory'));
  for (const selector of ['.name .ellipsis','.size','.mtime','.perms']) await expect(page.locator('.sftp-row').filter({has:page.locator('.nav')}).first().locator(selector).first()).toHaveCSS('direction','ltr');
  await shot('sftp');
});

test('Database stages a cell edit for review and discards it without writing', async ({page}) => {
  let writes = 0;
  await page.route('**/db/query', r => {
    if (!r.request().postDataJSON().statement.startsWith('SELECT')) writes++;
    return r.fulfill({json:{columns:[{name:'id',type_hint:'INT'},{name:'customer',type_hint:'VARCHAR'}],rows:[[1,'Customer before']],stats:{duration_ms:12,row_count:1},truncated:false}});
  });
  await page.route('**/db/object', r => r.fulfill({json:{primary_key:['id'],columns:[{name:'id',type_name:'INT',nullable:false},{name:'customer',type_name:'VARCHAR',nullable:true}],foreign_keys:[],indexes:[],name:'customers',kind:'table'}}));
  await setup(page,'database');
  await page.route('**/db/capabilities',r => r.fulfill({json:{engine:'mysql',query_language:'sql',version:'8.0',sql:true,can_write:true,explain:true}}));
  // Refresh the page so the capabilities advertise SQL editing.
  await page.reload();
  await expect(page.getByRole('tablist',{name:'Query tabs'})).toBeVisible();
  await page.locator('.qe-edit .cm-content').fill('SELECT id, customer FROM fixture.customers LIMIT 20');
  await page.getByRole('button',{name:/^Run\s+⌘/}).click();
  await expect(page.getByText('Customer before',{exact:true})).toBeVisible();
  await expect(page.locator('.gt-edit-hint')).toBeVisible();
  await page.getByText('Customer before',{exact:true}).dblclick();
  await page.locator('.cell-input').fill('Customer changed');
  await page.locator('.cell-input').press('Enter');
  await expect(page.getByTestId('pending-edits-bar')).toContainText('1 pending change');
  await page.getByTestId('pending-edits-bar').getByRole('button',{name:'Discard',exact:true}).click();
  await expect(page.getByText('Customer before',{exact:true})).toBeVisible();
  expect(writes).toBe(0);
});

test('SSH connection form retains a draft through delayed save failure', async ({page}) => {
  await setup(page,'database');
  await page.getByRole('button',{name:'New connection',exact:true}).first().click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('button',{name:'SSH',exact:true}).click();
  await dialog.getByLabel('Name',{exact:true}).fill('Private bastion draft');
  await dialog.getByLabel('Host',{exact:true}).fill('fixture.invalid');
  let release!: () => void;
  const held = new Promise<void>(resolve => release = resolve);
  await page.route('**/workspaces/*/connections', async r => {
    if (r.request().method() !== 'POST') return r.continue();
    await held;
    return r.fulfill({status:503,json:{code:'upstream',message:'Fixture save unavailable'}});
  });
  await dialog.getByRole('button',{name:'Create connection',exact:true}).click();
  await expect(dialog.getByRole('button',{name:'Saving…',exact:true})).toBeDisabled();
  release();
  await expect(page.getByText('Fixture save unavailable',{exact:true})).toBeVisible();
  await expect(dialog.getByLabel('Name',{exact:true})).toHaveValue('Private bastion draft');
  await expect(dialog.getByLabel('Host',{exact:true})).toHaveValue('fixture.invalid');
  await expect(dialog.getByRole('button',{name:'Create connection',exact:true})).toBeEnabled();
});

test('Kafka topic search opens a failed detail with recovery', async ({page}) => {
  await page.route('**/topics',r => r.fulfill({json:[{name:'customer-notifications',partitions:3,replication_factor:2,message_count:10245,cleanup_policy:'delete',internal:false},{name:'audit-events',partitions:1,replication_factor:2,message_count:19,cleanup_policy:'delete',internal:false}]}));
  await page.route('**/topics/stats',r => r.fulfill({json:{}}));
  await page.route('**/topics/customer-notifications',r => r.fulfill({status:503,json:{code:'upstream',message:'Topic details unavailable'}}));
  await setup(page,'broker');
  await page.locator('.tabs button',{hasText:'Topics'}).click();
  await page.getByLabel('Search topics').fill('customer');
  await expect(page.locator('.grid tbody tr')).toHaveCount(1);
  await page.getByText('customer-notifications',{exact:true}).click();
  await expect(page.getByText('Topic details unavailable',{exact:true})).toBeVisible();
  await expect(page.getByRole('button',{name:'Retry',exact:true})).toBeVisible();
});

test('Replay confirms the exact valid selector and displays fixture evidence', async ({page}) => {
  let selector: unknown;
  await page.route('**/brokers/clusters/*/replay',r => {
    selector = r.request().postDataJSON().selector;
    return r.fulfill({json:{replay_id:'fixture-replay',source_topic:'orders-dlq',target_topic:'orders',count:1,evidence:[{partition:0,offset:10,key_preview:'order-1',target_partition:0,target_offset:22}]}});
  });
  await setup(page,'broker');
  await page.locator('.tabs button',{hasText:'Replay'}).click();
  await page.getByLabel('Source topic').fill('orders-dlq');
  await page.getByLabel('Target topic').fill('orders');
  await page.getByLabel('Count',{exact:true}).fill('1');
  await page.getByRole('button',{name:'Replay',exact:true}).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('orders-dlq');
  await expect(dialog).toContainText('orders');
  await dialog.getByRole('button',{name:'Replay',exact:true}).click();
  await expect(page.locator('.evidence')).toContainText('order-1');
  expect(selector).toEqual({type:'latest',count:1});
});
